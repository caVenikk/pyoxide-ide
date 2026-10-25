//! The password prompt of a data source whose password is not kept
//! ("Save: Never"): a small modal dialog with the user and a password
//! field, asked on every connect. The answer goes to the caller only; the
//! prompt keeps nothing.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, IntoElement, Render,
    WeakEntity, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

use db_core::Secret;

use crate::password::PasswordField;

ui::ui_commands! {
    COMMANDS, scope = "DataSourcePasswordPrompt";
    /// Type the password.
    PromptSet { value: String } => "db.password-prompt.set", "Set Password",
        "Types `value` into the password prompt of a data source (tests and the control channel; the value is never echoed or logged).",
        r#"{"value": "secret"}"#;
    /// Connect with the password.
    PromptOk => "db.password-prompt.ok", "OK",
        "Connects with the password typed into the prompt.";
    /// Do not connect.
    PromptCancel => "db.password-prompt.cancel", "Cancel",
        "Closes the password prompt without connecting.";
}

/// Texts (our own).
const TITLE: &str = "Connect to Data Source";
const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 150.0;
const LABEL_WIDTH: f32 = 80.0;

type Done = Box<dyn FnOnce(Option<Secret>, &mut App)>;

#[derive(Default)]
struct OpenPrompt(Option<WeakEntity<Prompt>>);

impl Global for OpenPrompt {}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// The open prompt's state (never the password), `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenPrompt>()
        .and_then(|p| p.0.as_ref()?.upgrade())
        .map(|p| p.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Asks for the password of the data source `name` (user `user`) over
/// `window`; `done` gets it, or `None` when the prompt was cancelled.
pub fn ask(
    name: &str,
    user: &str,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<Secret>, &mut App) + 'static,
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
    let (name, user) = (name.to_owned(), user.to_owned());
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let prompt = cx.new(|cx| Prompt::new(name, user, done, window, cx));
        built = Some(prompt.clone());
        prompt
    });
    if let Some(prompt) = built {
        cx.set_global(OpenPrompt(Some(prompt.downgrade())));
    }
}

struct Prompt {
    name: String,
    user: String,
    field: Entity<PasswordField>,
    done: Rc<RefCell<Option<Done>>>,
    focus: FocusHandle,
}

impl Focusable for Prompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Prompt {
    fn new(
        name: String,
        user: String,
        done: Rc<RefCell<Option<Done>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| PasswordField::new("password-prompt-field", cx));
        window.focus(&field.focus_handle(cx));
        Self {
            name,
            user,
            field,
            done,
            focus: cx.focus_handle(),
        }
    }

    fn finish(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let secret = ok.then(|| self.field.read(cx).secret().unwrap_or_default());
        let done = self.done.borrow_mut().take();
        cx.set_global(OpenPrompt(None));
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
        if let Some(done) = done {
            cx.defer(move |cx| done(secret, cx));
        }
    }

    fn state_json(&self, cx: &App) -> Json {
        json!({
            "name": self.name,
            "user": self.user,
            "field": self.field.read(cx).state_json(),
        })
    }
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let row = |label: &'static str, value: gpui::AnyElement| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(div().w(px(LABEL_WIDTH)).flex_none().child(label))
                .child(div().flex_1().child(value))
        };
        let bar = BottomBar::new()
            .button(BarButton::new(
                "password-prompt-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(PromptOk),
            ))
            .button(BarButton::new(
                "password-prompt-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(PromptCancel),
            ))
            .render(cx);
        div()
            .id("password-prompt")
            .key_context("DataSourcePasswordPrompt")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, a: &PromptSet, _, cx| {
                this.field.update(cx, |f, cx| f.set(&a.value, cx));
            }))
            .on_action(cx.listener(|this, _: &PromptOk, window, cx| this.finish(true, window, cx)))
            .on_action(
                cx.listener(|this, _: &PromptCancel, window, cx| this.finish(false, window, cx)),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "enter" => this.finish(true, window, cx),
                    "escape" => this.finish(false, window, cx),
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
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .pt(px(12.0))
                    .child(
                        div()
                            .text_color(theme.color(UiRole::TextMuted))
                            .child(format!("Data source \u{201c}{}\u{201d}", self.name)),
                    )
                    .child(row(
                        "User:",
                        div().child(self.user.clone()).into_any_element(),
                    ))
                    .child(row("Password:", self.field.clone().into_any_element())),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
