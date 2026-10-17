//! The breakpoint properties popup (the reference's editor balloon:
//! `DebuggerUIUtil.showXBreakpointEditorBalloon`,
//! `XLightBreakpointPropertiesPanel`, `XSuspendPolicyPanel`,
//! `BreakpointEditor`): under a breakpoint's icon, pointing at it; the
//! breakpoint's name, Enabled, Suspend with All and Thread (the type's
//! default in bold, Make Default while another is chosen), the condition
//! (a label, or a check box when the breakpoint has one) in a field with
//! its history, "More (⇧⌘F8)" and Done. Enabled and Suspend apply at once,
//! the condition when the popup closes (Done, Escape, Enter, a click
//! outside, More).

use debugger::{BreakpointId, Debugger, LineBreakpoint, SuspendPolicy};
use editor::Editor;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    FontWeight, IntoElement, ParentElement as _, PathBuilder, Render, SharedString, Styled as _,
    Subscription, Window, anchored, canvas, deferred, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::checkbox;
use ui::form::{CHECK_GAP, MenuPopup, field_button, radio};
use ui::icons::IconName;
use ui::menu::MenuItem;
use ui::theme::ThemeExt as _;

use crate::breakpoints::commands::{
    PopupDone, PopupHistory, PopupMore, PopupSetCondition, PopupToggle, ViewBreakpoints,
};
use crate::breakpoints::presentation;
use crate::view::DebuggerView;

/// The key context of the popup.
pub const CONTEXT: &str = "BreakpointPopup";
/// The popup's content width (the light panel's 400 px spacer).
const CONTENT_WIDTH: f32 = 400.0;
/// The space around the content and between its rows.
const INSET: f32 = 12.0;
const ROW_GAP: f32 = 10.0;
/// The pointer at the icon: its width and height.
const POINTER_W: f32 = 12.0;
const POINTER_H: f32 = 6.0;
/// From the line's bottom to the pointer's tip.
const POINTER_GAP: f32 = 2.0;

/// What the popup asks of its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupEvent {
    Done,
    More,
}

/// The popup's view.
pub struct BreakpointPopup {
    debugger: Debugger,
    pub(crate) id: BreakpointId,
    title: String,
    pub(crate) condition: Entity<Editor>,
    /// The breakpoint had a condition when the popup opened: the
    /// condition's check box is shown instead of its label.
    pub(crate) condition_check: bool,
    pub(crate) condition_on: bool,
    /// The policy chosen with the radio buttons (kept while Suspend is off).
    policy: SuspendPolicy,
    history: Vec<String>,
    menu: Option<MenuPopup>,
    focus: FocusHandle,
    activate: bool,
}

impl EventEmitter<PopupEvent> for BreakpointPopup {}

impl BreakpointPopup {
    fn new(
        debugger: Debugger,
        b: &LineBreakpoint,
        history: Vec<String>,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = b.condition.clone().unwrap_or_default();
        let condition = editor::field::new(&text, None, false, window, cx);
        let policy = match b.suspend {
            SuspendPolicy::None => debugger.default_suspend(),
            policy => policy,
        };
        Self {
            title: presentation::short_text(&b.file, b.line),
            condition_check: b.condition.is_some(),
            condition_on: true,
            policy,
            debugger,
            id: b.id,
            condition,
            history,
            menu: None,
            focus: cx.focus_handle(),
            activate,
        }
    }

    /// The condition as the popup leaves it: `None` when it is empty or
    /// unchecked.
    pub(crate) fn condition_text(&self, cx: &App) -> Option<String> {
        let text = self.condition.read(cx).text();
        let text = text.trim();
        (!text.is_empty() && (!self.condition_check || self.condition_on)).then(|| text.to_owned())
    }

    fn toggle(&mut self, a: &PopupToggle, _: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.debugger.breakpoints().get(self.id).cloned() else {
            return;
        };
        let suspends = b.suspend != SuspendPolicy::None;
        match a.option.as_str() {
            "enabled" => {
                self.debugger
                    .update_breakpoint(self.id, |b| b.enabled = !b.enabled);
            }
            "suspend" => {
                let policy = if suspends {
                    SuspendPolicy::None
                } else {
                    self.policy
                };
                self.debugger
                    .update_breakpoint(self.id, |b| b.suspend = policy);
            }
            "all" | "thread" if suspends => {
                self.policy = if a.option == "all" {
                    SuspendPolicy::All
                } else {
                    SuspendPolicy::Thread
                };
                let policy = self.policy;
                self.debugger
                    .update_breakpoint(self.id, |b| b.suspend = policy);
            }
            "make-default" if suspends => self.debugger.set_default_suspend(self.policy),
            "condition" if self.condition_check => self.condition_on = !self.condition_on,
            _ => {}
        }
        cx.notify();
    }

    fn done(&mut self, _: &PopupDone, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PopupEvent::Done);
    }

    fn more(&mut self, _: &PopupMore, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PopupEvent::More);
    }

    /// The conditions entered before, under the field.
    fn history(&mut self, _: &PopupHistory, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_empty() {
            return;
        }
        let items = self
            .history
            .iter()
            .map(|c| MenuItem::action(c.clone(), PopupSetCondition { text: c.clone() }))
            .collect();
        let field = self.condition.read(cx).text_area_bounds();
        let position = field.map_or(point(px(0.0), px(0.0)), |b| b.bottom_left());
        let focus = self.condition.focus_handle(cx);
        self.menu = MenuPopup::open(
            items,
            None,
            position,
            &focus,
            self.activate,
            window,
            cx,
            |this: &mut Self, cx| {
                if let Some(menu) = this.menu.take() {
                    menu.close(cx);
                }
            },
        );
    }

    fn set_condition(
        &mut self,
        a: &PopupSetCondition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = self.menu.take() {
            menu.close(cx);
        }
        self.condition
            .update(cx, |e, cx| e.set_text_selected(&a.text, cx));
        self.condition_on = true;
        window.focus(&self.condition.focus_handle(cx));
        cx.notify();
    }

    pub(crate) fn state_json(&self, cx: &App) -> Json {
        let b = self.debugger.breakpoints().get(self.id).cloned();
        json!({
            "title": self.title,
            "enabled": b.as_ref().map(|b| b.enabled),
            "suspend": b.as_ref().map(|b| b.suspend != SuspendPolicy::None),
            "policy": self.policy.as_str(),
            "default_policy": self.debugger.default_suspend().as_str(),
            "make_default": b.as_ref().is_some_and(|b| b.suspend != SuspendPolicy::None)
                && self.policy != self.debugger.default_suspend(),
            "condition": self.condition.read(cx).text(),
            "condition_check": self.condition_check,
            "condition_on": self.condition_on,
            "history": self.history,
        })
    }
}

impl Render for BreakpointPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let b = self.debugger.breakpoints().get(self.id).cloned();
        let enabled = b.as_ref().is_some_and(|b| b.enabled);
        let suspends = b.as_ref().is_some_and(|b| b.suspend != SuspendPolicy::None);
        let default = self.debugger.default_suspend();
        let thread = self
            .debugger
            .provider()
            .line_breakpoint_type()
            .is_none_or(|t| t.suspend_thread);
        let disabled = theme.color(UiRole::TextDisabled);
        let toggle = |option: &'static str| {
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                window.dispatch_action(
                    Box::new(PopupToggle {
                        option: option.to_owned(),
                    }),
                    cx,
                )
            }
        };
        let check = |id: &'static str, checked: bool, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(CHECK_GAP))
                .child(checkbox((id, 0usize), checked, cx))
                .child(label)
        };
        let radio_choice = |id: &'static str, policy: SuspendPolicy, label: &'static str| {
            let bold = policy == default;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(CHECK_GAP))
                .child(radio((id, 0usize), self.policy == policy, cx))
                .child(
                    div()
                        .when(bold, |d| d.font_weight(FontWeight::BOLD))
                        .when(!suspends, |d| d.text_color(disabled))
                        .child(label),
                )
                .when(suspends, |d| {
                    d.on_click(toggle(if policy == SuspendPolicy::All {
                        "all"
                    } else {
                        "thread"
                    }))
                })
        };
        let more_label: SharedString = {
            let shortcut = ui::keys::shortcut_in(&PopupMore, &self.focus, window)
                .or_else(|| ui::keys::shortcut_in(&ViewBreakpoints, &self.focus, window));
            match shortcut {
                Some(s) => format!("More ({s})").into(),
                None => "More".into(),
            }
        };
        let condition_label = if self.condition_check {
            check("bp-popup-condition", self.condition_on, "Condition:")
                .on_click(toggle("condition"))
                .into_any_element()
        } else {
            div().child("Condition:").into_any_element()
        };
        let field_enabled = !self.condition_check || self.condition_on;
        let field = editor::field::frame(&self.condition, false, false, window, cx)
            .when(!field_enabled, |d| {
                d.opacity(ui::components::DISABLED_ALPHA)
            })
            .child(field_button(
                "bp-popup-expand",
                IconName::EXPAND,
                "Expand",
                None,
            ))
            .child(field_button(
                "bp-popup-history",
                IconName::CHEVRON_DOWN,
                "History",
                (!self.history.is_empty()).then(|| Box::new(PopupHistory) as Box<dyn gpui::Action>),
            ));
        let make_default = (thread && suspends && self.policy != default).then(|| {
            ui::components::text_button("bp-popup-make-default", "Make Default", true, cx)
                .debug_selector(|| "bp-popup-make-default".into())
                .on_click(toggle("make-default"))
        });
        let done = div()
            .id("bp-popup-done")
            .debug_selector(|| "bp-popup-done".into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(ui::form::BUTTON_HEIGHT))
            .min_w(px(ui::form::BUTTON_WIDTH))
            .px(px(14.0))
            .rounded(ui::form::field_radius(cx))
            .bg(theme.color(UiRole::Accent))
            .text_color(theme.color(UiRole::OnAccent))
            .child("Done")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(PopupDone), cx));
        div()
            .id("breakpoint-popup")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::toggle))
            .on_action(cx.listener(Self::done))
            .on_action(cx.listener(Self::more))
            .on_action(cx.listener(Self::history))
            .on_action(cx.listener(Self::set_condition))
            .w(px(CONTENT_WIDTH))
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .child(
                div()
                    .id("bp-popup-title")
                    .debug_selector(|| "bp-popup-title".into())
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .child(self.title.clone()),
            )
            .child(check("bp-popup-enabled", enabled, "Enabled").on_click(toggle("enabled")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(14.0))
                    .child(
                        check(
                            "bp-popup-suspend",
                            suspends,
                            if thread {
                                "Suspend:"
                            } else {
                                "Suspend execution"
                            },
                        )
                        .on_click(toggle("suspend")),
                    )
                    .when(thread, |d| {
                        d.child(radio_choice("bp-popup-all", SuspendPolicy::All, "All"))
                            .child(radio_choice(
                                "bp-popup-thread",
                                SuspendPolicy::Thread,
                                "Thread",
                            ))
                    })
                    .child(div().flex_1())
                    .children(make_default),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(condition_label)
                    .child(field),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(
                        ui::link::link("bp-popup-more", more_label, cx).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(PopupMore), cx),
                        ),
                    )
                    .child(div().flex_1())
                    .child(done),
            )
    }
}

/// The popup while it shows.
pub(crate) struct OpenPopup {
    pub(crate) id: BreakpointId,
    pub(crate) editor: Entity<Editor>,
    /// The line (zero-based) of the icon it points at.
    pub(crate) line: usize,
    pub(crate) view: Entity<BreakpointPopup>,
    _events: Subscription,
}

impl DebuggerView {
    /// Opens the properties popup of a line breakpoint under its icon in an
    /// editor, the focus in the condition field.
    pub(crate) fn open_popup(
        &mut self,
        editor: &Entity<Editor>,
        id: BreakpointId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popup(true, window, cx);
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        let Some(b) = debugger.breakpoints().get(id).cloned() else {
            return;
        };
        let line = b.line as usize - 1;
        let history = self.history.clone();
        let activate = self.activate;
        let view = cx.new(|cx| BreakpointPopup::new(debugger, &b, history, activate, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            PopupEvent::Done => this.close_popup(true, window, cx),
            PopupEvent::More => {
                let id = this.popup.as_ref().map(|p| p.id);
                this.close_popup(true, window, cx);
                this.open_dialog(id, window, cx);
            }
        });
        let field = view.read(cx).condition.focus_handle(cx);
        window.focus(&field);
        self.tooltip = None;
        self.popup = Some(OpenPopup {
            id,
            editor: editor.clone(),
            line,
            view,
            _events: events,
        });
        cx.notify();
    }

    /// Closes the popup; with `save` its condition is kept, and the focus
    /// goes back to the editor.
    pub fn close_popup(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.popup.take() else {
            return;
        };
        if let Some(menu) = p.view.update(cx, |v, _| v.menu.take()) {
            menu.close(cx);
        }
        if save && let Some(debugger) = self.debugger.clone() {
            let condition = p.view.read(cx).condition_text(cx);
            let changed = debugger
                .breakpoints()
                .get(p.id)
                .is_some_and(|b| b.condition != condition);
            if changed {
                debugger.update_breakpoint(p.id, |b| b.condition = condition.clone());
            }
            if let Some(condition) = &condition {
                self.remember_condition(condition);
            }
            if let Some(path) = p.editor.read(cx).path().cloned() {
                self.refresh_path(&path, cx);
            }
        }
        if p.view.read(cx).focus.contains_focused(window, cx)
            || p.view
                .read(cx)
                .condition
                .focus_handle(cx)
                .is_focused(window)
        {
            window.focus(&p.editor.focus_handle(cx));
        }
        cx.notify();
    }

    /// The popup at its place under the icon in the active editor; not
    /// drawn while its line is scrolled out of view.
    pub(crate) fn render_popup(
        &self,
        active: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = self.popup.as_ref()?;
        if active != Some(&p.editor) {
            return None;
        }
        let cell = p.editor.read(cx).line_number_bounds(p.line)?;
        let theme = cx.theme();
        let width = px(CONTENT_WIDTH + 2.0 * INSET + 2.0);
        // The icon is at the right of the line numbers' cell (4 px gap,
        // 14 px icon).
        let icon_x = cell.right() - px(11.0);
        let viewport = window.viewport_size();
        let left = (icon_x - width / 2.0)
            .min(viewport.width - width - px(8.0))
            .max(px(8.0));
        let pointer_left = icon_x - left - px(POINTER_W / 2.0);
        let fill_color = theme.color(UiRole::SurfaceRaised);
        let border_color = theme.color(UiRole::PopupBorder);
        let pointer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let (x0, x1) = (bounds.left(), bounds.left() + px(POINTER_W));
                let mid = bounds.left() + px(POINTER_W / 2.0);
                let (base, tip) = (bounds.bottom() + px(1.0), bounds.top());
                let mut fill = PathBuilder::fill();
                fill.move_to(point(x0, base));
                fill.line_to(point(mid, tip));
                fill.line_to(point(x1, base));
                fill.close();
                if let Ok(path) = fill.build() {
                    window.paint_path(path, fill_color);
                }
                let mut edge = PathBuilder::stroke(px(1.0));
                edge.move_to(point(x0, base));
                edge.line_to(point(mid, tip));
                edge.line_to(point(x1, base));
                if let Ok(path) = edge.build() {
                    window.paint_path(path, border_color);
                }
            },
        )
        .ml(pointer_left)
        .w(px(POINTER_W))
        .h(px(POINTER_H));
        let this = cx.entity().downgrade();
        Some(
            deferred(
                anchored()
                    .position(point(left, cell.bottom() + px(POINTER_GAP)))
                    .child(
                        div()
                            .id("breakpoint-popup-host")
                            .occlude()
                            .flex()
                            .flex_col()
                            .on_mouse_down_out(move |_, window, cx| {
                                let _ = this.update(cx, |v, cx| v.close_popup(true, window, cx));
                            })
                            // The pointer joins the box over its border.
                            .child(pointer)
                            .child(
                                div()
                                    .w(width)
                                    .p(px(INSET))
                                    .rounded(theme.metric(themes::Metric::CornerRadius) * 2.0)
                                    .border_1()
                                    .border_color(border_color)
                                    .bg(fill_color)
                                    .shadow(ui::components::popup_shadow())
                                    .child(p.view.clone()),
                            ),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    pub(crate) fn popup_json(&self, cx: &App) -> Json {
        match &self.popup {
            Some(p) => {
                let mut state = p.view.read(cx).state_json(cx);
                state["line"] = json!(p.line + 1);
                state["file"] = json!(p.editor.read(cx).path().map(|path| self.shown_path(path)));
                state
            }
            None => Json::Null,
        }
    }
}
