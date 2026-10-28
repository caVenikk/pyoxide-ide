//! The question asked when an action would end a transaction that holds
//! changes (switching to auto-commit, closing a session, disconnecting):
//! commit them, roll them back or leave things as they are. The session
//! layer asks with `DecisionNeeded` and does nothing until the action is
//! repeated with the answer; this window collects it.

use std::cell::RefCell;
use std::rc::Rc;

use db_session::TxPolicy;
use gpui::{
    App, Context, FocusHandle, Focusable, Global, IntoElement, Render, WeakEntity, Window, div,
    prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

ui::ui_commands! {
    COMMANDS, scope = "TxDecision";
    /// Commit the pending changes.
    DecisionCommit => "db.tx-decision.commit", "Commit",
        "Answers the pending-changes question with Commit: the action goes on after the changes are committed.";
    /// Roll the pending changes back.
    DecisionRollback => "db.tx-decision.rollback", "Rollback",
        "Answers the pending-changes question with Rollback: the action goes on after the changes are rolled back.";
    /// Leave everything as it is.
    DecisionCancel => "db.tx-decision.cancel", "Cancel",
        "Closes the pending-changes question; the action is dropped and the changes stay pending.";
}

const TITLE: &str = "Pending Changes";
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 150.0;

type Done = Box<dyn FnOnce(Option<TxPolicy>, &mut App)>;

#[derive(Default)]
struct OpenDecision(Option<WeakEntity<Decision>>);

impl Global for OpenDecision {}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// The open question's state, `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenDecision>()
        .and_then(|d| d.0.as_ref()?.upgrade())
        .map(|d| d.read(cx).state_json())
        .unwrap_or(Json::Null)
}

/// Asks over `window` what to do with the pending changes described by
/// `message`; `done` gets the policy to repeat the action with, or `None`
/// when the question was cancelled.
pub(crate) fn ask(
    message: String,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<TxPolicy>, &mut App) + 'static,
) {
    let options = ui::dialog_window::DialogOptions {
        title: TITLE.into(),
        size: gpui::size(
            px(WIDTH),
            px(HEIGHT + ui::dialog_window::title_height(true)),
        ),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let decision = cx.new(|cx| Decision::new(message, done, window, cx));
        built = Some(decision.clone());
        decision
    });
    if let Some(decision) = built {
        cx.set_global(OpenDecision(Some(decision.downgrade())));
    }
}

struct Decision {
    message: String,
    done: Rc<RefCell<Option<Done>>>,
    focus: FocusHandle,
}

impl Focusable for Decision {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Decision {
    fn new(
        message: String,
        done: Rc<RefCell<Option<Done>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            message,
            done,
            focus,
        }
    }

    fn finish(&mut self, policy: Option<TxPolicy>, window: &mut Window, cx: &mut Context<Self>) {
        let done = self.done.borrow_mut().take();
        cx.set_global(OpenDecision(None));
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
        if let Some(done) = done {
            cx.defer(move |cx| done(policy, cx));
        }
    }

    fn state_json(&self) -> Json {
        json!({"message": self.message, "buttons": ["Commit", "Rollback", "Cancel"]})
    }
}

impl Render for Decision {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bar = BottomBar::new()
            .button(BarButton::new(
                "tx-decision-commit",
                "Commit",
                ButtonRole::Ok,
                Box::new(DecisionCommit),
            ))
            .button(
                BarButton::new(
                    "tx-decision-rollback",
                    "Rollback",
                    ButtonRole::Other,
                    Box::new(DecisionRollback),
                )
                .primary(false),
            )
            .button(BarButton::new(
                "tx-decision-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(DecisionCancel),
            ))
            .render(cx);
        div()
            .id("tx-decision")
            .key_context("TxDecision")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &DecisionCommit, window, cx| {
                this.finish(Some(TxPolicy::Commit), window, cx)
            }))
            .on_action(cx.listener(|this, _: &DecisionRollback, window, cx| {
                this.finish(Some(TxPolicy::Rollback), window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DecisionCancel, window, cx| this.finish(None, window, cx)),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "enter" => this.finish(Some(TxPolicy::Commit), window, cx),
                    "escape" => this.finish(None, window, cx),
                    _ => {}
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .text_size(theme.ui_size())
            .font_family(theme.ui_family())
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
            .child(
                div()
                    .flex_1()
                    .px(px(16.0))
                    .pt(px(14.0))
                    .child(self.message.clone()),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
