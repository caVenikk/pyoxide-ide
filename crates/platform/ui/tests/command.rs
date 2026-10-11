//! UI commands declared with `ui_commands!`, invoked by id in a headless
//! window.

use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, Render, TestAppContext, Window, div,
    prelude::*,
};
use serde_json::json;

ui::ui_commands! {
    COMMANDS, scope = "Counter";
    /// Adds one.
    Increment => "counter.increment", "Increment", "Adds one to the counter.";
    Add { amount: i64 } => "counter.add", "Add", "Adds an amount.", r#"{"amount": 2}"#;
    State => "counter.state", "Counter State", "Returns the counter.";
}

struct Counter {
    value: i64,
    focus: FocusHandle,
}

impl Focusable for Counter {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Counter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Counter")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Increment, _, cx| {
                this.value += 1;
                cx.notify();
            }))
            .on_action(cx.listener(|this, a: &Add, _, cx| {
                this.value += a.amount;
                cx.notify();
            }))
            .size_full()
    }
}

#[gpui::test]
fn commands_run_by_id(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        ui::command::register(cx, COMMANDS);
        cx.open_window(Default::default(), |window, cx| {
            let view = cx.new(|cx| Counter {
                value: 0,
                focus: cx.focus_handle(),
            });
            window.focus(&view.read(cx).focus.clone());
            view
        })
        .unwrap()
    });
    let view = window.root(cx).unwrap();
    cx.update(|cx| {
        let entity = view.clone();
        ui::command::register_query(
            cx,
            "counter.state",
            std::rc::Rc::new(move |_, _, cx| Ok(json!({ "value": entity.read(cx).value }))),
        );
    });
    cx.run_until_parked();

    let invoke = |cx: &mut TestAppContext, id: &str, args: serde_json::Value| {
        cx.update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
    };

    assert_eq!(
        invoke(cx, "counter.increment", json!(null)),
        Ok(json!(null))
    );
    assert_eq!(
        invoke(cx, "counter.add", json!({"amount": 5})),
        Ok(json!(null))
    );
    cx.run_until_parked();
    assert_eq!(
        invoke(cx, "counter.state", json!(null)),
        Ok(json!({"value": 6}))
    );

    let err = invoke(cx, "counter.add", json!({"amount": "x"})).unwrap_err();
    assert_eq!(err.code, "invalid_args");
    let err = invoke(cx, "counter.nope", json!(null)).unwrap_err();
    assert_eq!(err.code, "not_found");

    let ids: Vec<_> = cx.update(|cx| {
        ui::command::list(cx)
            .into_iter()
            .map(|c| c.meta.id)
            .collect()
    });
    assert_eq!(ids, ["counter.add", "counter.increment", "counter.state"]);
    assert_eq!(COMMANDS[1].meta.args_example, r#"{"amount": 2}"#);
    assert_eq!(COMMANDS[0].meta.args_example, "{}");
}

#[gpui::test]
fn unavailable_commands_report_an_error(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        ui::command::register(cx, COMMANDS);
        cx.open_window(Default::default(), |_, cx| cx.new(|_| Empty))
            .unwrap()
    });
    cx.run_until_parked();
    let err = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke("counter.increment", json!(null), window, cx)
        })
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "not_available");
}

struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
