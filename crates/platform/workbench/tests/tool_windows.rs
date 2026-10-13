//! The tool window below the editor (Run), the product hooks of the main
//! window (status bar widgets, init) and the events it reports.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Focusable as _, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window,
    WindowHandle, div, point, px,
};
use serde_json::{Value, json};
use workbench::workbench::ToolWindow;
use workbench::{Product, StatusWidget, Workbench, WorkbenchEvent};

fn open(cx: &mut TestAppContext, product: Product) -> WindowHandle<Workbench> {
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    draw(cx, window);
    window
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
    out
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &'static str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    VisualTestContext::from_window(window.into(), cx).debug_bounds(selector)
}

fn focused(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Option<ToolWindow> {
    cx.update_window(window.into(), |root, window, cx| {
        let workbench = root.downcast::<Workbench>().unwrap();
        workbench.read(cx).focused_tool_window(window, cx)
    })
    .unwrap()
}

fn test_product() -> Product {
    Product {
        name: "Test IDE",
        ..Product::default()
    }
}

/// The Run tool window opens below the editor and spans under the project
/// tool window (the reference IDE's default layout); the first command
/// focuses it, the second one hides it.
#[gpui::test]
fn the_run_tool_window_spans_below_the_editor(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    // (Debug bounds outlive the frames that drew them: after hiding, the
    // absence is checked through the state.)
    assert!(bounds(cx, window, "run-tool-window").is_none());
    let editor_before = bounds(cx, window, "editor-area").unwrap();
    assert!(bounds(cx, window, "stripe-run").is_some());

    invoke(cx, window, "workbench.toggle-run", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(true));
    assert_eq!(focused(cx, window), Some(ToolWindow::Run));
    let run = bounds(cx, window, "run-tool-window").unwrap();
    let project = bounds(cx, window, "project-tool-window").unwrap();
    let editor = bounds(cx, window, "editor-area").unwrap();
    assert_eq!(run.left(), project.left());
    assert_eq!(run.right(), editor.right());
    assert!(run.top() > editor.bottom());
    assert!(run.top() > project.bottom());
    assert!(editor.size.height < editor_before.size.height);
    // A third of the space between the toolbar and the status bar.
    let status = bounds(cx, window, "status-bar").unwrap();
    let toolbar = bounds(cx, window, "toolbar").unwrap();
    let pane = status.top() - toolbar.bottom();
    assert!((run.size.height - pane * 0.33).abs() < px(1.0), "{run:?}");

    invoke(cx, window, "workbench.toggle-run", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(false));
}

/// Dragging the gap above the Run tool window resizes it.
#[gpui::test]
fn the_run_tool_window_is_resized_by_its_splitter(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    invoke(cx, window, "workbench.toggle-run", json!(null));
    let before = bounds(cx, window, "run-tool-window").unwrap();
    let splitter = bounds(cx, window, "bottom-splitter").unwrap();
    let from = splitter.center();
    let to = point(from.x, from.y - px(100.0));
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(from, None, Modifiers::none());
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    draw(cx, window);
    let after = bounds(cx, window, "run-tool-window").unwrap();
    assert!(
        (after.size.height - before.size.height - px(100.0)).abs() < px(1.0),
        "{before:?} -> {after:?}"
    );
    assert_eq!(after.bottom(), before.bottom());
}

/// Hide Active Tool Window hides the tool window that has the focus.
#[gpui::test]
fn hide_active_tool_window_hides_the_focused_one(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    invoke(cx, window, "workbench.toggle-run", json!(null));
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(false));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));

    invoke(cx, window, "workbench.toggle-run", json!(null));
    invoke(cx, window, "workbench.toggle-project", json!(null));
    assert_eq!(focused(cx, window), Some(ToolWindow::Project));
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(false));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(true));
}

/// A product's status bar widget is created with the workbench and sees
/// its events; the product's init runs at the end of the workbench's.
#[gpui::test]
fn product_hooks_see_the_workbench(cx: &mut TestAppContext) {
    struct Widget {
        text: String,
    }
    impl Render for Widget {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "test-widget".into())
                .w(px(40.0))
                .child(self.text.clone())
        }
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let product = Product {
        name: "Test IDE",
        init: Some(|cx| cx.set_global(InitRan)),
        status_widgets: vec![StatusWidget {
            id: "test-widget",
            create: Rc::new(move |workbench, _, cx| {
                let sink = sink.clone();
                let workbench = workbench.upgrade().expect("the workbench exists");
                cx.new(|cx| {
                    cx.subscribe(&workbench, move |this: &mut Widget, _, event, cx| {
                        sink.borrow_mut().push(event.clone());
                        if let WorkbenchEvent::ActiveFileChanged(file) = event {
                            this.text = file
                                .as_ref()
                                .and_then(|f| f.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            cx.notify();
                        }
                    })
                    .detach();
                    Widget {
                        text: String::new(),
                    }
                })
                .into()
            }),
        }],
        ..Product::default()
    };
    let window = open(cx, product);
    assert!(cx.update(|cx| cx.has_global::<InitRan>()));
    assert!(bounds(cx, window, "test-widget").is_some());

    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.py"), "a = 1\n").unwrap();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    invoke(cx, window, "project.open-file", json!({"path": "a.py"}));
    let (project_root, active) = window
        .read_with(cx, |w, cx| (w.project_root(cx), w.active_file(cx)))
        .unwrap();
    assert_eq!(project_root, Some(root.clone()));
    assert_eq!(active, Some(root.join("a.py")));
    invoke(cx, window, "tabs.close", json!(null));
    assert_eq!(
        *events.borrow(),
        [
            WorkbenchEvent::ProjectOpened(root.clone()),
            WorkbenchEvent::ActiveFileChanged(Some(root.join("a.py"))),
            WorkbenchEvent::ActiveFileChanged(None),
        ]
    );
}

struct InitRan;
impl gpui::Global for InitRan {}

/// The Run stripe button shows and focuses the Run tool window, and hides
/// it when it is shown.
#[gpui::test]
fn the_run_stripe_button_toggles_the_tool_window(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    let button = bounds(cx, window, "stripe-run").unwrap().center();
    let click = |cx: &mut TestAppContext| {
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.simulate_click(button, Modifiers::none());
        draw(cx, window);
    };
    click(cx);
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(true));
    let run = window.read_with(cx, |w, _| w.run_window().clone()).unwrap();
    let run_focused = cx
        .update_window(window.into(), |_, window, cx| {
            run.focus_handle(cx).contains_focused(window, cx)
        })
        .unwrap();
    assert!(run_focused);
    click(cx);
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(false));
}
