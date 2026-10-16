//! The balloons of the main window: the host of `ui::notification`'s
//! stack, its timer, the input that starts it, and `workbench.notify`.
//!
//! The stack sits at the bottom right of the window, 10 px left of the
//! right stripe and above the status bar (the reference's
//! `ActionCenterBalloonLayout`), over the tool windows and below popups
//! and dialogs. An action link runs its command in the window and closes
//! the balloon.
//!
//! Every notification also goes into the timeline that the Notifications
//! tool window lists ([`tool_window`]), until "Clear all".

mod tool_window;

pub use tool_window::{ID, NotificationsToolWindow};

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Context, IntoElement, KeyDownEvent, MouseDownEvent, MouseMoveEvent, Render,
    ScrollWheelEvent, Subscription, Task, WeakEntity, Window, canvas, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::Value;
use themes::Metric;
use ui::notification::{
    BalloonEvent, BalloonStack, MARGIN, Notification, NotificationAction, NotificationKind,
    render_stack,
};
use ui::theme::ThemeExt;

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Clear the Notifications tool window.
    ClearAllNotifications => "notifications.clear-all", "Clear All Notifications",
        "Removes every notification from the Notifications tool window's timeline and closes the balloons shown (`ClearAllNotifications`).";
    /// Show a balloon.
    Notify {
        kind: Option<String>,
        title: Option<String>,
        content: Option<String>,
        sticky: Option<bool>,
        actions: Option<Vec<NotifyAction>>,
    } => "workbench.notify", "Show Notification",
        "Shows a notification balloon: kind information, warning or error (errors stay 5 minutes, others 10 seconds unless sticky), a title, a content text and action links that run commands.",
        r#"{"kind": "warning", "title": "Low memory", "content": "The IDE is running low on memory.", "actions": [{"text": "Analyze", "command": "workbench.state"}]}"#;
}

/// An action link of [`Notify`].
#[derive(Clone, PartialEq, Debug, Deserialize, gpui::private::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(crate = "gpui::private::schemars")]
pub struct NotifyAction {
    /// The link's text.
    pub text: String,
    /// The command it runs.
    pub command: String,
    /// The command's arguments.
    pub args: Option<Value>,
}

impl Notify {
    /// The notification the command describes.
    pub fn notification(&self) -> Notification {
        let kind = self
            .kind
            .as_deref()
            .and_then(NotificationKind::from_name)
            .unwrap_or_default();
        let mut n = Notification::new(
            kind,
            self.title.clone().unwrap_or_default(),
            self.content.clone().unwrap_or_default(),
        );
        if let Some(sticky) = self.sticky {
            n.sticky = sticky;
        }
        n.actions = self
            .actions
            .iter()
            .flatten()
            .map(|a| NotificationAction {
                text: a.text.clone().into(),
                command: a.command.clone(),
                args: a.args.clone().unwrap_or(Value::Null),
            })
            .collect();
        n
    }
}

/// A notification of the timeline.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    /// The id of its balloon.
    pub id: u64,
    pub notification: Notification,
    /// When it came.
    pub time: chrono::DateTime<chrono::Local>,
}

impl TimelineEntry {
    /// The time as the timeline shows it (`NotificationGroupComponent`'s
    /// `formatPrettyDateTime`): the time of day today, "Yesterday", or
    /// the date.
    pub fn time_text(&self, now: chrono::DateTime<chrono::Local>) -> String {
        let day = self.time.date_naive();
        let today = now.date_naive();
        if day == today {
            self.time.format("%H:%M").to_string()
        } else if today.pred_opt() == Some(day) {
            "Yesterday".to_owned()
        } else {
            self.time.format("%Y-%m-%d").to_string()
        }
    }
}

/// The balloons of a main window and the timeline of its notifications.
pub struct Notifications {
    stack: BalloonStack,
    /// Oldest first.
    timeline: Vec<TimelineEntry>,
    timer: Task<()>,
    _activation: Subscription,
}

impl Notifications {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let now = cx.background_executor().now();
            this.stack.set_active(window.is_window_active(), now);
            this.schedule(cx);
        });
        Self {
            stack: BalloonStack::default(),
            timeline: Vec::new(),
            timer: Task::ready(()),
            _activation: activation,
        }
    }

    pub fn stack(&self) -> &BalloonStack {
        &self.stack
    }

    /// The notifications of the timeline, oldest first.
    pub fn timeline(&self) -> &[TimelineEntry] {
        &self.timeline
    }

    /// Shows a balloon and adds the notification to the timeline; returns
    /// its id.
    pub fn notify(&mut self, notification: Notification, cx: &mut Context<Self>) -> u64 {
        let id = self.stack.add(notification.clone());
        self.timeline.push(TimelineEntry {
            id,
            notification,
            time: chrono::Local::now(),
        });
        cx.notify();
        id
    }

    /// Empties the timeline; the balloons go with it (the reference
    /// expires the notifications).
    pub fn clear_timeline(&mut self, cx: &mut Context<Self>) {
        self.timeline.clear();
        self.close_all(cx);
    }

    /// Closes a balloon.
    pub fn close(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.stack.close(id) {
            self.schedule(cx);
            cx.notify();
        }
    }

    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        self.stack.close_all();
        self.schedule(cx);
        cx.notify();
    }

    /// The balloons and the timeline for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Value {
        let mut state = self.stack.state_json(cx.background_executor().now());
        let now = chrono::Local::now();
        state["timeline"] = self
            .timeline
            .iter()
            .rev()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "kind": e.notification.kind.name(),
                    "title": e.notification.title.as_ref(),
                    "content": e.notification.content.as_ref(),
                    "actions": e.notification.actions.iter()
                        .map(|a| a.text.as_ref()).collect::<Vec<_>>(),
                    "time": e.time_text(now),
                })
            })
            .collect();
        state
    }

    fn now(cx: &App) -> Instant {
        cx.background_executor().now()
    }

    /// Keyboard or mouse input in the window.
    fn input(&mut self, cx: &mut Context<Self>) {
        let now = Self::now(cx);
        let before = self.stack.next_expiry(now);
        self.stack.input(now);
        if self.stack.next_expiry(now) != before {
            self.schedule(cx);
        }
    }

    /// Waits for the next balloon to go.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let Some(wait) = self.stack.next_expiry(Self::now(cx)) else {
            self.timer = Task::ready(());
            return;
        };
        self.timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                if this.stack.expire(Self::now(cx)) {
                    cx.notify();
                }
                this.schedule(cx);
            });
        });
    }

    fn event(&mut self, event: &BalloonEvent, window: &mut Window, cx: &mut Context<Self>) {
        let now = Self::now(cx);
        match event {
            BalloonEvent::Close { all: true, .. } => self.close_all(cx),
            BalloonEvent::Close { id, .. } => self.close(*id, cx),
            BalloonEvent::Action { id, index } => {
                let action = self
                    .stack
                    .get(*id)
                    .and_then(|b| b.notification.actions.get(*index))
                    .cloned();
                self.close(*id, cx);
                if let Some(action) = action {
                    let (command, args) = (action.command, action.args);
                    // After this update: the command may reach the
                    // workbench, which holds this entity.
                    window.defer(cx, move |window, cx| {
                        if let Err(e) = ui::command::invoke(&command, args, window, cx) {
                            crate::env::marker(&format!("notification action {command}: {e:?}"));
                        }
                    });
                }
            }
            BalloonEvent::Hover { id, hovered } => {
                self.stack.set_hovered(*id, *hovered, now);
                self.schedule(cx);
                cx.notify();
            }
            BalloonEvent::HoverAction { id, index } => {
                self.stack.set_hovered_action(*id, *index);
                cx.notify();
            }
            BalloonEvent::Expand { id } => {
                self.stack.toggle_expanded(*id);
                cx.notify();
            }
            // And the Notifications tool window opens
            // (`ActionCenterBalloonLayout`: `closeAll`, `showLog`).
            BalloonEvent::ShowMore => {
                self.close_all(cx);
                window.defer(cx, |window, cx| {
                    window.dispatch_action(
                        Box::new(crate::commands::ActivateToolWindow {
                            id: tool_window::ID.to_owned(),
                        }),
                        cx,
                    )
                });
            }
        }
    }
}

/// Registers input listeners for this frame: the first input after a
/// balloon is shown starts its time.
fn listen_for_input(this: WeakEntity<Notifications>, window: &mut Window) {
    fn input(this: &WeakEntity<Notifications>, cx: &mut App) {
        if let Some(this) = this.upgrade() {
            this.update(cx, |this, cx| this.input(cx));
        }
    }
    let t = this.clone();
    window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture {
            input(&t, cx);
        }
    });
    let t = this.clone();
    window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture {
            input(&t, cx);
        }
    });
    let t = this.clone();
    window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture {
            input(&t, cx);
        }
    });
    window.on_key_event(move |_: &KeyDownEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture {
            input(&this, cx);
        }
    });
}

impl Render for Notifications {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.stack.wrap(cx);
        let this = cx.entity().downgrade();
        let handler: ui::notification::BalloonHandler = Rc::new(move |event, window, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.event(event, window, cx));
            }
        });
        let Some(stack) = render_stack(&self.stack, handler, cx) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let right = theme.metric(Metric::StripeWidth) + px(MARGIN);
        let bottom = theme.metric(Metric::StatusBarHeight) + px(MARGIN);
        let input = cx.entity().downgrade();
        div()
            .absolute()
            .right(right)
            .bottom(bottom)
            .child(stack)
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| listen_for_input(input, window),
                )
                .absolute()
                .size_0(),
            )
            .into_any_element()
    }
}
