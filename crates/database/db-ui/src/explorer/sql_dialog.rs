//! The modal dialogs of the tree's actions that change the database or a
//! data source: Drop (the statements, with IF EXISTS and CASCADE options),
//! Truncate (the statement) and Rename (the new name, and for a database
//! object the statement). The SQL shows before it runs; OK hands it to the
//! caller, which runs it on a session of the data source. The texts are
//! our own (the reference's database plugin is closed).

use std::cell::RefCell;
use std::rc::Rc;

use editor::Editor;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, IntoElement, Render,
    SharedString, WeakEntity, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

use crate::ddl::DropOptions;

ui::ui_commands! {
    COMMANDS, scope = "DatabaseSqlDialog";
    /// Run the statements.
    SqlDialogOk => "db.sql-dialog.ok", "OK",
        "Runs the statements shown in the open Drop, Truncate or Rename dialog of the Database tool window (Rename: with the name typed).";
    /// Close without running.
    SqlDialogCancel => "db.sql-dialog.cancel", "Cancel",
        "Closes the open Drop, Truncate or Rename dialog of the Database tool window without running anything.";
    /// Toggle an option of the Drop dialog.
    SqlDialogToggle { option: String } => "db.sql-dialog.toggle", "Toggle Option",
        "Toggles an option of the open Drop dialog: `if-exists` or `cascade`; the statements shown follow.",
        r#"{"option": "cascade"}"#;
    /// Type the new name.
    SqlDialogSetName { value: String } => "db.sql-dialog.set-name", "Set Name",
        "Types `value` as the new name in the open Rename dialog.",
        r#"{"value": "clients"}"#;
}

/// Texts (our own).
const WIDTH: f32 = 480.0;
const IF_EXISTS: &str = "Use IF EXISTS syntax";
const CASCADE: &str = "Use DROP CASCADE syntax";
const PREVIEW: &str = "Preview";

/// What the dialog asks.
pub(crate) enum Ask {
    /// Drop: the statements by options.
    Drop(Box<dyn Fn(DropOptions) -> Vec<String>>),
    /// Truncate: the statements.
    Confirm(Vec<String>),
    /// Rename: the old name, and the statements for a new name (`None`
    /// for a data source, which is renamed without SQL).
    Rename {
        old: String,
        statements: Option<RenameStatements>,
    },
}

/// The statements for a new name.
pub(crate) type RenameStatements = Box<dyn Fn(&str) -> Vec<String>>;

/// The answer: the statements to run, and for Rename the new name.
pub(crate) struct Answer {
    pub statements: Vec<String>,
    pub name: String,
}

type Done = Box<dyn FnOnce(Answer, &mut App)>;

#[derive(Default)]
struct OpenSqlDialog(Option<WeakEntity<SqlDialog>>);

impl Global for OpenSqlDialog {}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// The open dialog's state, `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenSqlDialog>()
        .and_then(|d| d.0.as_ref()?.upgrade())
        .map(|d| d.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Opens the dialog over `window`: `title`, a line of `message` above the
/// fields, then what `ask` asks; `done` gets the answer of OK.
pub(crate) fn open(
    title: &str,
    message: String,
    ask: Ask,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Answer, &mut App) + 'static,
) {
    let height = match &ask {
        Ask::Drop(_) => 330.0,
        Ask::Confirm(_) => 260.0,
        Ask::Rename {
            statements: Some(_),
            ..
        } => 290.0,
        Ask::Rename { .. } => 150.0,
    };
    let options = ui::dialog_window::DialogOptions {
        title: SharedString::from(title.to_owned()),
        size: gpui::size(
            px(WIDTH),
            px(height + ui::dialog_window::title_height(true)),
        ),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let title = title.to_owned();
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| SqlDialog::new(title, message, ask, done, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        cx.set_global(OpenSqlDialog(Some(dialog.downgrade())));
    }
}

pub(crate) struct SqlDialog {
    title: String,
    message: String,
    ask: Ask,
    options: DropOptions,
    name: Option<Entity<Editor>>,
    done: Rc<RefCell<Option<Done>>>,
    focus: FocusHandle,
    _subscription: Option<gpui::Subscription>,
}

impl Focusable for SqlDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SqlDialog {
    fn new(
        title: String,
        message: String,
        ask: Ask,
        done: Rc<RefCell<Option<Done>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let mut subscription = None;
        let name = match &ask {
            Ask::Rename { old, .. } => {
                let field = editor::field::new(old, None, true, window, cx);
                field.update(cx, |e, cx| e.set_text_selected(old, cx));
                window.focus(&field.focus_handle(cx));
                subscription =
                    Some(cx.subscribe(&field, |_, _, _: &editor::EditorEvent, cx| cx.notify()));
                Some(field)
            }
            _ => {
                window.focus(&focus);
                None
            }
        };
        Self {
            title,
            message,
            ask,
            options: DropOptions::default(),
            name,
            done,
            focus,
            _subscription: subscription,
        }
    }

    fn typed_name(&self, cx: &App) -> String {
        self.name
            .as_ref()
            .map(|n| n.read(cx).text().trim().to_owned())
            .unwrap_or_default()
    }

    /// The statements as the dialog shows them now.
    fn statements(&self, cx: &App) -> Vec<String> {
        match &self.ask {
            Ask::Drop(make) => make(self.options),
            Ask::Confirm(statements) => statements.clone(),
            Ask::Rename {
                old,
                statements: Some(make),
            } => {
                let name = self.typed_name(cx);
                if name.is_empty() || name == *old {
                    Vec::new()
                } else {
                    make(&name)
                }
            }
            Ask::Rename { .. } => Vec::new(),
        }
    }

    /// OK can run: a rename to another, non-empty name.
    fn can_finish(&self, cx: &App) -> bool {
        match &self.ask {
            Ask::Rename { old, .. } => {
                let name = self.typed_name(cx);
                !name.is_empty() && name != *old
            }
            _ => true,
        }
    }

    fn finish(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        if ok && !self.can_finish(cx) {
            return;
        }
        let answer = ok.then(|| Answer {
            statements: self.statements(cx),
            name: self.typed_name(cx),
        });
        let done = self.done.borrow_mut().take();
        cx.set_global(OpenSqlDialog(None));
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
        if let (Some(done), Some(answer)) = (done, answer) {
            cx.defer(move |cx| done(answer, cx));
        }
    }

    fn toggle(&mut self, option: &str, cx: &mut Context<Self>) {
        match option {
            "if-exists" => self.options.if_exists = !self.options.if_exists,
            "cascade" => self.options.cascade = !self.options.cascade,
            _ => return,
        }
        cx.notify();
    }

    fn state_json(&self, cx: &App) -> Json {
        json!({
            "title": self.title,
            "message": self.message,
            "statements": self.statements(cx),
            "name": self.name.as_ref().map(|_| self.typed_name(cx)),
            "if_exists": self.options.if_exists,
            "cascade": self.options.cascade,
            "ok_enabled": self.can_finish(cx),
        })
    }

    fn option_row(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        ui::form::check_row(id, (id, 1usize), checked, div().child(label), cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle(id, cx);
            }))
            .into_any_element()
    }

    fn preview(&self, cx: &App) -> gpui::AnyElement {
        let theme = cx.theme();
        let text = self.statements(cx).join("\n");
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(6.0))
            .child(
                div()
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(PREVIEW),
            )
            .child(
                div()
                    .id("sql-dialog-preview")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(theme.color(UiRole::Border))
                    .bg(theme.color(UiRole::EditorBackground))
                    .font_family(theme.mono_family())
                    .text_size(theme.editor_size())
                    .whitespace_normal()
                    .child(text),
            )
            .into_any_element()
    }
}

impl Render for SqlDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ok_enabled = self.can_finish(cx);
        let bar = BottomBar::new()
            .button(
                BarButton::new("sql-dialog-ok", "OK", ButtonRole::Ok, Box::new(SqlDialogOk))
                    .enabled(ok_enabled),
            )
            .button(BarButton::new(
                "sql-dialog-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(SqlDialogCancel),
            ))
            .render(cx);
        let mut body = div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .child(div().child(self.message.clone()));
        match &self.ask {
            Ask::Drop(_) => {
                body = body
                    .child(self.option_row("if-exists", IF_EXISTS, self.options.if_exists, cx))
                    .child(self.option_row("cascade", CASCADE, self.options.cascade, cx))
                    .child(self.preview(cx));
            }
            Ask::Confirm(_) => body = body.child(self.preview(cx)),
            Ask::Rename { statements, .. } => {
                if let Some(name) = &self.name {
                    body = body.child(editor::field::frame(name, true, false, window, cx));
                }
                if statements.is_some() {
                    body = body.child(self.preview(cx));
                }
            }
        }
        let theme = cx.theme();
        div()
            .id("sql-dialog")
            .key_context("DatabaseSqlDialog")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &SqlDialogOk, window, cx| this.finish(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SqlDialogCancel, window, cx| this.finish(false, window, cx)),
            )
            .on_action(cx.listener(|this, a: &SqlDialogToggle, _, cx| this.toggle(&a.option, cx)))
            .on_action(cx.listener(|this, a: &SqlDialogSetName, _, cx| {
                if let Some(name) = &this.name {
                    name.update(cx, |e, cx| e.set_text_selected(&a.value, cx));
                }
                cx.notify();
            }))
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
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(body)
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
