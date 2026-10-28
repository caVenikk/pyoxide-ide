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
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, Global, IntoElement,
    Pixels, Render, ScrollHandle, SharedString, StyledText, TextRun, WeakEntity, Window, canvas,
    div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use sql_syntax::{Dialect, TokenKind};
use themes::{Metric, TextRole, UiRole};
use ui::form::{BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::icons::{Icon, IconName};
use ui::menu::MenuItem;
use ui::theme::{Theme, ThemeExt};

use crate::ddl::{DropOptions, Qualify};

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
        "Toggles an option of the open Drop or Truncate dialog: `if-exists`, `cascade`, `reformat` or `preview` (the Preview section); the statements shown follow.",
        r#"{"option": "cascade"}"#;
    /// Choose how the Drop or Truncate dialog qualifies names.
    SqlDialogSetQualify { value: String } => "db.sql-dialog.set-qualify", "Set Qualify Objects",
        "Chooses \"Qualify objects with schema names\" in the open Drop or Truncate dialog: `Auto`, `Always` or `Never`; the statements shown follow.",
        r#"{"value": "never"}"#;
    /// Type the new name.
    SqlDialogSetName { value: String } => "db.sql-dialog.set-name", "Set Name",
        "Types `value` as the new name in the open Rename dialog.",
        r#"{"value": "clients"}"#;
}

/// Texts (our own).
const WIDTH: f32 = 345.0;
const QUALIFY: &str = "Qualify objects with schema names:";
const IF_EXISTS: &str = "Use IF EXISTS syntax";
const CASCADE: &str = "Use DROP CASCADE syntax";
const REFORMAT: &str = "Reformat generated code";
const PREVIEW: &str = "Preview";
/// The sides of the dialog's content, and the height of the preview box.
const SIDE: f32 = 12.0;
const PREVIEW_BOX: f32 = 120.0;
/// The width of the combo box of the qualifying.
const COMBO_WIDTH: f32 = 100.0;

/// What the dialog asks.
pub(crate) enum Ask {
    /// Drop: the statements by options.
    Drop(Box<dyn Fn(DropOptions) -> Vec<String>>),
    /// Truncate: the statements by the qualifying.
    Confirm(Box<dyn Fn(DropOptions) -> Vec<String>>),
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

/// Opens the dialog over `window`: `title`, then what `ask` asks (Rename:
/// the line of `message` above its field); the statements are shown in
/// `dialect`'s SQL; `done` gets the answer of OK.
pub(crate) fn open(
    title: &str,
    message: String,
    dialect: Dialect,
    ask: Ask,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Answer, &mut App) + 'static,
) {
    let height = match &ask {
        Ask::Drop(_) => 335.0,
        Ask::Confirm(_) => 250.0,
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
        let dialog = cx.new(|cx| SqlDialog::new(title, message, dialect, ask, done, window, cx));
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
    dialect: Dialect,
    ask: Ask,
    options: DropOptions,
    reformat: bool,
    preview_open: bool,
    qualify_anchor: Rc<RefCell<Option<Bounds<Pixels>>>>,
    qualify_list: Option<MenuPopup>,
    preview_scroll: ScrollHandle,
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
        dialect: Dialect,
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
            dialect,
            ask,
            options: DropOptions::default(),
            reformat: true,
            preview_open: true,
            qualify_anchor: Rc::default(),
            qualify_list: None,
            preview_scroll: ScrollHandle::new(),
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
            Ask::Drop(make) | Ask::Confirm(make) => make(self.options),
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
        self.close_list(cx);
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
            "reformat" => self.reformat = !self.reformat,
            "preview" => self.preview_open = !self.preview_open,
            _ => return,
        }
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.qualify_list.take() {
            list.close(cx);
        }
    }

    fn set_qualify(&mut self, value: &str, cx: &mut Context<Self>) {
        self.close_list(cx);
        if let Some(qualify) = Qualify::from_label(value) {
            self.options.qualify = qualify;
        }
        cx.notify();
    }

    /// The list of the combo box under it.
    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.qualify_list.is_some() {
            self.close_list(cx);
            return;
        }
        let items = Qualify::ALL
            .into_iter()
            .map(|q| {
                MenuItem::action(
                    q.label(),
                    SqlDialogSetQualify {
                        value: q.label().to_owned(),
                    },
                )
                .bold(q == self.options.qualify)
            })
            .collect();
        let position = match *self.qualify_anchor.borrow() {
            Some(b) => gpui::point(b.left(), b.bottom() + px(2.0)),
            None => gpui::point(px(180.0), px(80.0)),
        };
        self.qualify_list = MenuPopup::open(
            items,
            None,
            position,
            &self.focus,
            !workbench::env::background(),
            window,
            cx,
            |this, cx| this.close_list(cx),
        );
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
            "qualify": self.options.qualify.label(),
            "reformat": self.reformat,
            "preview_open": self.preview_open,
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

    /// "Qualify objects with schema names:" and its combo box.
    fn qualify_row(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let anchor = self.qualify_anchor.clone();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(div().flex_none().whitespace_nowrap().child(QUALIFY))
            .child(
                ui::form::combo(
                    "sql-dialog-qualify",
                    vec![div().child(self.options.qualify.label()).into_any_element()],
                    cx,
                )
                .w(px(COMBO_WIDTH))
                .debug_selector(|| "sql-dialog-qualify".into())
                .child(
                    canvas(
                        move |b, _, _| {
                            *anchor.borrow_mut() = Some(b);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_list(window, cx))),
            )
            .into_any_element()
    }

    /// The collapsible "Preview" section: the titled separator with the
    /// chevron and the Open in Editor icon, and the statements in SQL.
    fn preview(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let open = self.preview_open;
        let header = div()
            .id("sql-dialog-preview-header")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .id("sql-dialog-preview-toggle")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(Icon::new(if open {
                        IconName::CHEVRON_DOWN
                    } else {
                        IconName::CHEVRON_RIGHT
                    }))
                    .child(div().whitespace_nowrap().child(PREVIEW))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle("preview", cx))),
            )
            .child(
                div()
                    .flex_1()
                    .ml(px(2.0))
                    .h(px(1.0))
                    .bg(theme.color(UiRole::Divider)),
            )
            // The statements open in an editor of their own in a later change.
            .child(ui::form::field_button(
                "sql-dialog-open-in-editor",
                IconName::EXTERNAL_LINK,
                "Open in Editor",
                None,
            ));
        let mut section = div().flex().flex_col().gap(px(6.0)).child(header);
        if open {
            section = section.child(self.preview_box(cx));
        }
        section.into_any_element()
    }

    /// The statements in the editor's font and the SQL colours, with the
    /// vertical scroll bar's area at the right.
    fn preview_box(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let text = self.statements(cx).join("\n");
        let runs = sql_runs(&text, self.dialect, theme);
        let handle = &self.preview_scroll;
        let track = theme.metric(Metric::ScrollbarWidth);
        let viewport: f32 = handle.bounds().size.height.into();
        let range: f32 = handle.max_offset().height.into();
        // The thumb fills the track while the text fits.
        let (thumb_top, thumb_height) = if viewport > 0.0 && range > 0.0 {
            let height = (viewport * viewport / (viewport + range)).max(20.0);
            let top = (-f32::from(handle.offset().y) / range).clamp(0.0, 1.0) * (viewport - height);
            (px(top), Some(px(height)))
        } else {
            (px(0.0), None)
        };
        div()
            .flex()
            .flex_row()
            .h(px(PREVIEW_BOX))
            .flex_none()
            .rounded(px(4.0))
            .border_1()
            .border_color(theme.color(UiRole::Border))
            .bg(theme.color(UiRole::EditorBackground))
            .overflow_hidden()
            .child(
                div()
                    .id("sql-dialog-preview")
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(handle)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_color(theme.text_color(TextRole::Variable))
                    .font_family(theme.mono_family())
                    .text_size(theme.editor_size())
                    .child(StyledText::new(SharedString::from(text)).with_runs(runs)),
            )
            .child(
                div()
                    .flex_none()
                    .relative()
                    .w(track)
                    .h_full()
                    .bg(theme.color(UiRole::ScrollbarTrack))
                    .child(
                        div()
                            .absolute()
                            .top(thumb_top)
                            .left(px(1.0))
                            .right(px(1.0))
                            .when_some(thumb_height, |d, h| d.h(h))
                            .when(thumb_height.is_none(), |d| d.bottom_0())
                            .rounded(px(3.0))
                            .bg(theme.color(UiRole::Scrollbar)),
                    ),
            )
            .into_any_element()
    }
}

/// The colour runs of `text` as SQL of `dialect`.
fn sql_runs(text: &str, dialect: Dialect, theme: &Theme) -> Vec<TextRun> {
    let font = gpui::font(theme.mono_family());
    let run = |len: usize, role: TextRole| TextRun {
        len,
        font: font.clone(),
        color: theme.text_color(role),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs: Vec<TextRun> = Vec::new();
    let mut at = 0;
    for token in sql_syntax::lex(text, dialect) {
        if token.start > at {
            runs.push(run(token.start - at, TextRole::Variable));
        }
        let role = match token.kind {
            TokenKind::Word if dialect.keyword(token.text(text)).is_some() => TextRole::Keyword,
            TokenKind::LineComment | TokenKind::BlockComment => TextRole::Comment,
            TokenKind::String => TextRole::String,
            TokenKind::Number => TextRole::Number,
            TokenKind::Operator => TextRole::Operator,
            TokenKind::Semicolon | TokenKind::Comma | TokenKind::Dot => TextRole::Punctuation,
            TokenKind::LParen | TokenKind::RParen => TextRole::Bracket,
            _ => TextRole::Variable,
        };
        runs.push(run(token.end - token.start, role));
        at = token.end;
    }
    if at < text.len() {
        runs.push(run(text.len() - at, TextRole::Variable));
    }
    runs
}

impl Render for SqlDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ok_enabled = self.can_finish(cx);
        let bar = BottomBar::new()
            .separator(false)
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
            .px(px(SIDE))
            .pt(px(10.0))
            .pb(px(4.0));
        match &self.ask {
            Ask::Drop(_) => {
                body = body
                    .child(self.qualify_row(cx))
                    .child(self.option_row("if-exists", IF_EXISTS, self.options.if_exists, cx))
                    .child(self.option_row("cascade", CASCADE, self.options.cascade, cx))
                    .child(self.option_row("reformat", REFORMAT, self.reformat, cx))
                    .child(self.preview(cx));
            }
            Ask::Confirm(_) => {
                body = body
                    .child(self.qualify_row(cx))
                    .child(self.option_row("reformat", REFORMAT, self.reformat, cx))
                    .child(self.preview(cx));
            }
            Ask::Rename { statements, .. } => {
                body = body.child(div().child(self.message.clone()));
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
            .on_action(
                cx.listener(|this, a: &SqlDialogSetQualify, _, cx| this.set_qualify(&a.value, cx)),
            )
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
