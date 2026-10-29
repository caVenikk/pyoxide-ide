//! The password prompt of a data source whose password is not at hand (not
//! kept, or "Save: Never"): a small modal dialog with the user, a password
//! field and, when it asks to connect, the choice of how long the password
//! is kept. The answer goes to the caller only; the prompt keeps nothing.
//!
//! [`Asker`] is what the data source handles' connections ask through: a
//! connection that finds no password asks the UI thread, which opens the
//! prompt over the window of the project (one prompt at a time; what is
//! asked twice is asked once).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, IntoElement, Render,
    SharedString, WeakEntity, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

use db_core::{PasswordSave, Secret};

use crate::password::PasswordField;

ui::ui_commands! {
    COMMANDS, scope = "DataSourcePasswordPrompt";
    /// Type the password.
    PromptSet { value: String } => "db.password-prompt.set", "Set Password",
        "Types `value` into the password prompt of a data source (tests and the control channel; the value is never echoed or logged).",
        r#"{"value": "secret"}"#;
    /// Choose how long the password is kept.
    PromptSave { value: String } => "db.password-prompt.save", "Set Save Choice",
        "Chooses in the password prompt how long the typed password is kept: `never`, `until-restart` or `forever` (tests and the control channel).",
        r#"{"value": "never"}"#;
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
const HEIGHT_WITH_SAVE: f32 = 190.0;
const LABEL_WIDTH: f32 = 80.0;

/// What was typed, and for how long it is kept.
pub struct Answer {
    pub password: Secret,
    pub save: PasswordSave,
}

type Done = Box<dyn FnOnce(Option<Answer>, &mut App)>;

#[derive(Default)]
struct OpenPrompt(Option<WeakEntity<Prompt>>);

impl Global for OpenPrompt {}

/// A connection of the data source `source` of the project at `root` found
/// no password: `reply` gets whether one is at hand now.
pub(crate) struct Request {
    pub root: PathBuf,
    pub source: String,
    pub reply: oneshot::Sender<bool>,
}

/// Where the connections of the data source handles ask for a password
/// (any thread).
#[derive(Clone)]
pub(crate) struct Asker(mpsc::UnboundedSender<Request>);

impl Global for Asker {}

impl Asker {
    /// Waits until the person has typed the password of `source` (`true`),
    /// or has not (`false`: cancelled, or nobody to ask).
    pub async fn ask(&self, root: PathBuf, source: String) -> bool {
        let (reply, answer) = oneshot::channel();
        let request = Request {
            root,
            source,
            reply,
        };
        if self.0.unbounded_send(request).is_err() {
            return false;
        }
        answer.await.unwrap_or(false)
    }
}

/// The asker of the module (made with it).
pub(crate) fn asker(cx: &App) -> Option<Asker> {
    cx.try_global::<Asker>().cloned()
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    let (tx, mut rx) = mpsc::unbounded::<Request>();
    cx.set_global(Asker(tx));
    // One prompt at a time, in the order asked: a request for a password
    // that has been typed meanwhile finds it at hand and asks nothing.
    cx.spawn(async move |cx| {
        while let Some(request) = rx.next().await {
            let answered =
                cx.update(|cx| crate::explorer::ask_password(&request.root, &request.source, cx));
            let ok = match answered {
                Ok(Some(answer)) => answer.await.unwrap_or(false),
                _ => false,
            };
            let _ = request.reply.send(ok);
        }
    })
    .detach();
}

/// The open prompt's state (never the password), `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenPrompt>()
        .and_then(|p| p.0.as_ref()?.upgrade())
        .map(|p| p.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Asks for the password of the data source `name` (user `user`) over
/// `window`; `done` gets it, or `None` when the prompt was cancelled. For a
/// test of a connection, which keeps nothing.
pub fn ask(
    name: &str,
    user: &str,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<Secret>, &mut App) + 'static,
) {
    open(name, user, None, window, cx, move |answer, cx| {
        done(answer.map(|a| a.password), cx)
    });
}

/// Asks for the password to connect to the data source `name` (user
/// `user`) over `window`, with the choice of how long it is kept (`save`
/// is the one shown first); `done` gets the answer, or `None` when the
/// prompt was cancelled.
pub fn ask_to_connect(
    name: &str,
    user: &str,
    save: PasswordSave,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<Answer>, &mut App) + 'static,
) {
    open(name, user, Some(save), window, cx, done);
}

fn open(
    name: &str,
    user: &str,
    save: Option<PasswordSave>,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<Answer>, &mut App) + 'static,
) {
    let height = if save.is_some() {
        HEIGHT_WITH_SAVE
    } else {
        HEIGHT
    };
    let options = ui::dialog_window::DialogOptions {
        title: TITLE.into(),
        size: gpui::size(
            px(WIDTH),
            px(height + ui::dialog_window::title_height(true)),
        ),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let (name, user) = (name.to_owned(), user.to_owned());
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let prompt = cx.new(|cx| Prompt::new(name, user, save, done, window, cx));
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
    /// How long the password is kept, when the prompt offers the choice.
    save: Option<PasswordSave>,
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
        save: Option<PasswordSave>,
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
            save,
            done,
            focus: cx.focus_handle(),
        }
    }

    fn finish(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let answer = ok.then(|| Answer {
            password: self.field.read(cx).secret().unwrap_or_default(),
            save: self.save.unwrap_or_default(),
        });
        let done = self.done.borrow_mut().take();
        cx.set_global(OpenPrompt(None));
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
        if let Some(done) = done {
            cx.defer(move |cx| done(answer, cx));
        }
    }

    fn state_json(&self, cx: &App) -> Json {
        json!({
            "name": self.name,
            "user": self.user,
            "save": self.save.map(PasswordSave::id),
            "field": self.field.read(cx).state_json(),
        })
    }

    /// "Never / Until restart / Forever".
    fn save_choice(&self, save: PasswordSave, cx: &mut Context<Self>) -> gpui::AnyElement {
        let choices: Vec<SharedString> = PasswordSave::ALL
            .iter()
            .map(|s| SharedString::from(save_title(*s)))
            .collect();
        let selected = PasswordSave::ALL.iter().position(|s| *s == save);
        let this = cx.entity().downgrade();
        div()
            .flex()
            .flex_row()
            .child(ui::form::segmented(
                "password-prompt-save",
                &choices,
                selected,
                move |ix, _, cx| {
                    if let (Some(save), Some(this)) = (PasswordSave::ALL.get(ix), this.upgrade()) {
                        this.update(cx, |this, cx| {
                            this.save = Some(*save);
                            cx.notify();
                        });
                    }
                },
                cx,
            ))
            .into_any_element()
    }
}

/// How the choice names a length of keeping (as the Data Sources dialog's
/// list does).
fn save_title(save: PasswordSave) -> &'static str {
    match save {
        PasswordSave::Never => "Never",
        PasswordSave::UntilRestart => "Until restart",
        PasswordSave::Forever => "Forever",
    }
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let save_row = self.save.map(|save| (save, self.save_choice(save, cx)));
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
            .on_action(cx.listener(|this, a: &PromptSave, _, cx| {
                if let (Some(save), Some(_)) = (PasswordSave::from_id(&a.value), this.save) {
                    this.save = Some(save);
                    cx.notify();
                }
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
                    .child(row("Password:", self.field.clone().into_any_element()))
                    .children(save_row.map(|(_, choice)| row("Save:", choice))),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
