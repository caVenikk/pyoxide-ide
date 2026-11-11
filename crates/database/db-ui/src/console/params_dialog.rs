//! The Parameters dialog: one row for every value the statements ask for,
//! its name, the words before it in the statement and a field for the
//! value. Execute is available when every field has a value; the values
//! are remembered by the console for the next run.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use editor::Editor;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Global, IntoElement, Render, WeakEntity, Window,
    div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;

use super::params::Slot;

ui::ui_commands! {
    COMMANDS, scope = "ConsoleParameters";
    /// Type a value into the Parameters dialog.
    ParamsSet { name: String, value: String } => "db.params.set", "Set Parameter",
        "Types `value` into the field of the parameter `name` of the open Parameters dialog (tests and the control channel).",
        r#"{"name": "id", "value": "7"}"#;
    /// Run with the values typed.
    ParamsExecute => "db.params.execute", "Execute",
        "Runs the statements with the values typed into the Parameters dialog, if every parameter has one.";
    /// Close the dialog without running.
    ParamsClose => "db.params.close", "Close",
        "Closes the Parameters dialog without running anything.";
}

const TITLE: &str = "Parameters";
const WIDTH: f32 = 500.0;
const ROW: f32 = 36.0;
const CHROME: f32 = 92.0;
const LABEL_WIDTH: f32 = 120.0;

type Done = Box<dyn FnOnce(Option<HashMap<String, String>>, &mut App)>;

#[derive(Default)]
struct OpenParams(Option<WeakEntity<Params>>);

impl Global for OpenParams {}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// The open dialog's state, `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenParams>()
        .and_then(|p| p.0.as_ref()?.upgrade())
        .map(|p| p.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Asks for the values of `slots` over `window`, the fields filled with
/// what `remembered` has; `done` gets the values by slot key, or `None`
/// when the dialog was closed.
pub(crate) fn ask(
    slots: Vec<Slot>,
    remembered: &HashMap<String, String>,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(Option<HashMap<String, String>>, &mut App) + 'static,
) {
    let height = CHROME + ROW * slots.len() as f32;
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
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| Params::new(slots, remembered, done, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        cx.set_global(OpenParams(Some(dialog.downgrade())));
    }
}

struct Row {
    slot: Slot,
    field: Entity<Editor>,
}

struct Params {
    rows: Vec<Row>,
    done: Rc<RefCell<Option<Done>>>,
    focus: FocusHandle,
}

impl Focusable for Params {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Params {
    fn new(
        slots: Vec<Slot>,
        remembered: &HashMap<String, String>,
        done: Rc<RefCell<Option<Done>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows: Vec<Row> = slots
            .into_iter()
            .map(|slot| {
                let text = remembered.get(&slot.key).map_or("", String::as_str);
                let field = editor::field::new(text, None, false, window, cx);
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                Row { slot, field }
            })
            .collect();
        let focus = cx.focus_handle();
        match rows.first() {
            Some(first) => window.focus(&first.field.focus_handle(cx)),
            None => window.focus(&focus),
        }
        Self { rows, done, focus }
    }

    fn values(&self, cx: &App) -> HashMap<String, String> {
        self.rows
            .iter()
            .map(|r| (r.slot.key.clone(), r.field.read(cx).text()))
            .collect()
    }

    fn complete(&self, cx: &App) -> bool {
        self.rows
            .iter()
            .all(|r| !r.field.read(cx).text().trim().is_empty())
    }

    fn finish(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        if ok && !self.complete(cx) {
            return;
        }
        let values = ok.then(|| self.values(cx));
        let done = self.done.borrow_mut().take();
        cx.set_global(OpenParams(None));
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
        if let Some(done) = done {
            cx.defer(move |cx| done(values, cx));
        }
    }

    fn set(&mut self, name: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(row) = self
            .rows
            .iter()
            .find(|r| r.slot.key == name || r.slot.label == name)
        {
            row.field.update(cx, |e, cx| e.set_text_selected(value, cx));
        }
    }

    fn state_json(&self, cx: &App) -> Json {
        json!({
            "rows": self.rows.iter().map(|r| json!({
                "name": r.slot.label,
                "context": r.slot.context,
                "value": r.field.read(cx).text(),
            })).collect::<Vec<_>>(),
            "can_execute": self.complete(cx),
        })
    }
}

impl Render for Params {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let complete = self.complete(cx);
        let mono = theme.mono_family();
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                div()
                    .id(("params-row", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .h(px(ROW))
                    .child(
                        div()
                            .w(px(LABEL_WIDTH))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .italic()
                            .font_family(mono.clone())
                            .child(r.slot.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_color(muted)
                            .font_family(mono.clone())
                            .child(r.slot.context.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(editor::field::frame(&r.field, false, false, window, cx)),
                    )
            })
            .collect();
        let bar = BottomBar::new()
            .button(BarButton::new(
                "params-close",
                "Close",
                ButtonRole::Cancel,
                Box::new(ParamsClose),
            ))
            .button(
                BarButton::new(
                    "params-execute",
                    "Execute",
                    ButtonRole::Ok,
                    Box::new(ParamsExecute),
                )
                .primary(complete)
                .enabled(complete),
            )
            .render(cx);
        div()
            .id("console-params")
            .key_context("ConsoleParameters")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, a: &ParamsSet, _, cx| this.set(&a.name, &a.value, cx)))
            .on_action(
                cx.listener(|this, _: &ParamsExecute, window, cx| this.finish(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ParamsClose, window, cx| this.finish(false, window, cx)),
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
                    .px(px(16.0))
                    .pt(px(12.0))
                    .children(rows),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
