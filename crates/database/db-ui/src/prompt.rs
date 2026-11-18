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
    WeakEntity, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

use db_core::{Backend, PasswordSave, Profile, Secret, profile::AuthMethod};
use ui::form::MenuPopup;
use ui::menu::MenuItem;

use crate::password::PasswordField;

ui::ui_commands! {
    COMMANDS, scope = "DataSourcePasswordPrompt";
    /// Type the password.
    PromptSet { value: String } => "db.password-prompt.set", "Set Password",
        "Types `value` into the password prompt of a data source (tests and the control channel; the value is never echoed or logged).",
        r#"{"value": "secret"}"#;
    /// Edit the user.
    PromptUser { value: String } => "db.password-prompt.user", "Set Connection User",
        "Sets the user for this connection.";
    /// Choose authentication.
    PromptAuth { value: String } => "db.password-prompt.auth", "Set Connection Authentication",
        "Selects a supported authentication method.";
    /// Choose how long the password is kept.
    PromptSave { value: String } => "db.password-prompt.save", "Set Save Choice",
        "Chooses in the password prompt how long the typed password is kept: `never`, `until-restart` `for-session` or `forever` (tests and the control channel).",
        r#"{"value": "never"}"#;
    /// Open the credential store configuration.
    PromptSettings => "db.password-prompt.settings", "IDE Passwords Settings…",
        "Opens the password storage configuration in an independent editor window.";
    /// Connect with the password.
    PromptOk => "db.password-prompt.ok", "OK",
        "Connects with the password typed into the prompt.";
    /// Do not connect.
    PromptCancel => "db.password-prompt.cancel", "Cancel",
        "Closes the password prompt without connecting.";
}

const WIDTH: f32 = 411.0;
const HEIGHT: f32 = 227.0;

const LABEL_WIDTH: f32 = 102.0;

/// What was typed, and for how long it is kept.
#[derive(Clone)]
pub struct Answer {
    pub user: String,
    pub auth: AuthMethod,
    pub password: Secret,
    pub save: PasswordSave,
}

impl Answer {
    /// pgpass is resolved when opening the physical connection.
    pub fn password_override(&self) -> Option<Secret> {
        (self.auth != AuthMethod::Pgpass).then(|| self.password.clone())
    }

    pub fn apply(&self, profile: &mut Profile) {
        if self.auth == AuthMethod::None && profile.backend == Backend::OracleOdpi {
            profile.user.clear();
        } else {
            profile.user.clone_from(&self.user);
        }
        profile.gui.get_or_insert_with(Default::default).auth = self.auth;
    }
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
    pub session: crate::store::CredentialSession,
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
    pub async fn ask(
        &self,
        root: PathBuf,
        source: String,
        session: crate::store::CredentialSession,
    ) -> bool {
        let (reply, answer) = oneshot::channel();
        let request = Request {
            root,
            source,
            session,
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
            let answered = cx.update(|cx| {
                crate::explorer::ask_password(&request.root, &request.source, &request.session, cx)
            });
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

/// Ask over the project's window; connection tests omit the save policy.
pub(crate) fn open(
    source: &crate::DataSource,
    save: Option<PasswordSave>,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<Answer>, &mut App) + 'static,
) {
    let name = &source.name;
    let options = ui::dialog_window::DialogOptions {
        title: format!("Connecting to '{name}'…").into(),
        size: gpui::size(px(WIDTH), px(HEIGHT)),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));

    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let prompt = cx.new(|cx| Prompt::new(source, save, done, window, cx));
        built = Some(prompt.clone());
        prompt
    });
    if let Some(prompt) = built {
        cx.set_global(OpenPrompt(Some(prompt.downgrade())));
    }
}

struct Prompt {
    name: String,
    backend: Backend,
    user: Entity<editor::Editor>,
    auth: AuthMethod,
    popup: Option<MenuPopup>,
    settings_error: Option<String>,
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
        source: &crate::DataSource,
        save: Option<PasswordSave>,
        done: Rc<RefCell<Option<Done>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| PasswordField::new("password-prompt-field", cx));
        let user = &source.profile.user;
        let editor = editor::field::new(user, None, true, window, cx);
        editor.update(cx, |e, cx| e.set_text_selected(user, cx));
        window.focus(&editor.focus_handle(cx));
        let on_close = done.clone();
        cx.on_release(move |_, cx| {
            if let Some(done) = on_close.borrow_mut().take() {
                cx.defer(move |cx| done(None, cx));
            }
        })
        .detach();
        Self {
            name: source.name.clone(),
            backend: source.profile.backend,
            user: editor,
            auth: source.profile.gui_options().auth,
            popup: None,
            settings_error: None,
            field,
            save,
            done,
            focus: cx.focus_handle(),
        }
    }

    fn finish(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let user = self.user.read(cx).text();
        if ok
            && user.is_empty()
            && (self.auth == AuthMethod::UserPassword || self.backend == Backend::Postgres)
        {
            window.focus(&self.user.focus_handle(cx));
            return;
        }
        let answer = ok.then(|| Answer {
            user,
            auth: self.auth,
            password: if self.auth == AuthMethod::UserPassword {
                self.field.read(cx).secret().unwrap_or_default()
            } else {
                Secret::default()
            },
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
            "user": self.user.read(cx).text(),
            "auth": self.auth.id(),
            "save": self.save.map(PasswordSave::id),
            "field": self.field.read(cx).state_json(),
        })
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, initial)) = crate::store(cx).password_settings_file() else {
            return;
        };
        let mut owner = window.window_handle();
        while let Some(parent) = ui::dialog_window::parent_of(owner, cx) {
            owner = parent;
        }
        let Some(owner) = owner.downcast::<workbench::Workbench>() else {
            return;
        };
        let file = cx.background_spawn(async move {
            use std::io::{ErrorKind, Write as _};
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => file.write_all(initial.as_bytes())?,
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            Ok::<_, std::io::Error>(path)
        });
        cx.spawn(async move |this, cx| match file.await {
            Ok(path) => {
                let _ = owner.update(cx, |workbench, _, cx| workbench.open_file_window(path, cx));
            }
            Err(err) => {
                let _ = this.update(cx, |prompt, cx| {
                    prompt.settings_error = Some(format!("Cannot open password settings: {err}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn combo(&self, auth: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (id, label) = if auth {
            ("password-prompt-auth", self.auth.label())
        } else {
            (
                "password-prompt-save",
                save_title(self.save.unwrap_or_default()),
            )
        };
        let bounds = Rc::new(RefCell::new(gpui::Bounds::default()));
        let anchor = bounds.clone();
        ui::form::combo(id, vec![div().child(label).into_any_element()], cx)
            .debug_selector(move || id.to_owned())
            .child(
                gpui::canvas(move |b, _, _| *anchor.borrow_mut() = b, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(p) = this.popup.take() {
                    p.close(cx);
                }
                let mut items: Vec<MenuItem> = if auth {
                    AuthMethod::choices(this.backend)
                        .iter()
                        .map(|method| {
                            MenuItem::action(
                                method.label(),
                                PromptAuth {
                                    value: method.id().into(),
                                },
                            )
                        })
                        .collect()
                } else {
                    PasswordSave::ALL
                        .into_iter()
                        .map(|s| {
                            MenuItem::action(
                                save_title(s),
                                PromptSave {
                                    value: s.id().into(),
                                },
                            )
                        })
                        .collect()
                };
                if !auth && crate::store(cx).password_settings_file().is_some() {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::action("IDE Passwords Settings…", PromptSettings));
                }
                let b = *bounds.borrow();
                let selected = if auth {
                    AuthMethod::choices(this.backend)
                        .iter()
                        .position(|method| *method == this.auth)
                } else {
                    PasswordSave::ALL
                        .iter()
                        .position(|save| Some(*save) == this.save)
                };
                this.popup = MenuPopup::open_with(
                    items,
                    ui::menu::PopupOptions {
                        selected,
                        ..Default::default()
                    },
                    gpui::point(b.left(), b.bottom()),
                    &this.focus,
                    !workbench::env::background(),
                    window,
                    cx,
                    |this, cx| {
                        if let Some(p) = this.popup.take() {
                            p.close(cx);
                        }
                    },
                );
            }))
            .into_any_element()
    }
}

/// How the choice names a length of keeping (as the Data Sources dialog's
/// list does).
fn save_title(save: PasswordSave) -> &'static str {
    match save {
        PasswordSave::ForSession => "For session",
        PasswordSave::Never => "Never",
        PasswordSave::UntilRestart => "Until restart",
        PasswordSave::Forever => "Forever",
    }
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let auth = self.combo(true, cx);
        let save = self.save.map(|_| self.combo(false, cx));
        let uses_password = self.auth == AuthMethod::UserPassword;
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
            .separator(false)
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
            .on_action(cx.listener(|this, a: &PromptUser, _, cx| {
                this.user
                    .update(cx, |e, cx| e.set_text_selected(&a.value, cx))
            }))
            .on_action(cx.listener(|this, a: &PromptAuth, _, cx| {
                let Some(method) = AuthMethod::choices(this.backend)
                    .iter()
                    .find(|method| method.id() == a.value)
                else {
                    return;
                };
                this.auth = *method;
                cx.notify();
            }))
            .on_action(cx.listener(|this, a: &PromptSet, _, cx| {
                this.field.update(cx, |f, cx| f.set(&a.value, cx));
            }))
            .on_action(
                cx.listener(|this, _: &PromptSettings, window, cx| this.open_settings(window, cx)),
            )
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
            .capture_action(
                cx.listener(|this, _: &editor::commands::Newline, window, cx| {
                    this.finish(true, window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &editor::commands::Escape, window, cx| {
                    this.finish(false, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                let m = e.keystroke.modifiers;
                if m.control || m.platform || m.alt || m.function {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "enter" => this.finish(true, window, cx),
                    "escape" => this.finish(false, window, cx),
                    "tab" if m.shift => window.focus_prev(),
                    "tab" => window.focus_next(),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .text_size(theme.ui_size())
            .font_family(theme.ui_family())
            .children(ui::dialog_window::title_bar(
                format!("Connecting to '{}'…", self.name),
                true,
                cx,
            ))
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
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                ui::icons::Icon::new(ui::icons::IconName::WARNING).size(px(16.0)),
                            )
                            .child(
                                self.settings_error.clone().unwrap_or_else(|| {
                                    "Please provide database credentials.".into()
                                }),
                            ),
                    )
                    .child(row("Authentication:", auth))
                    .child(row(
                        "User:",
                        div()
                            .w(px(118.0))
                            .child(editor::field::frame(&self.user, true, false, window, cx))
                            .into_any_element(),
                    ))
                    .when(uses_password, |d| {
                        d.child(row(
                            "Password:",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .child(div().w(px(118.0)).child(self.field.clone()))
                                .children(save.map(|choice| {
                                    div()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child("Save:")
                                        .child(div().flex_1().child(choice))
                                }))
                                .into_any_element(),
                        ))
                    }),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
