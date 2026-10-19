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
use workbench::tool_windows::{Area, ToolWindowContent, ToolWindowSpec, ToolWindowView};
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
    // The Run tool window does not exist before its first activation.
    assert!(bounds(cx, window, "stripe-run").is_none());

    invoke(cx, window, "workbench.toggle-run", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(true));
    assert_eq!(focused(cx, window), Some(ToolWindow::Run));
    let run = bounds(cx, window, "run-tool-window").unwrap();
    assert!(bounds(cx, window, "stripe-run").is_some());
    assert!(bounds(cx, window, "run-empty").is_some(), "the empty state");
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
    let root = native_path::canonicalize(dir.path()).unwrap();
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
    // The first activation creates the tool window and its button.
    invoke(cx, window, "workbench.toggle-run", json!(null));
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(false));
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

/// A tool window of a test: an empty view with its own focus.
struct TestToolWindow {
    focus: gpui::FocusHandle,
}

impl Render for TestToolWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

impl gpui::Focusable for TestToolWindow {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl ToolWindowView for TestToolWindow {}

/// Registers a test tool window `id` in `area` and returns its view.
fn register(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &'static str,
    area: Area,
    registered: bool,
) -> gpui::Entity<TestToolWindow> {
    let view = window
        .update(cx, |w, _, cx| {
            let view = cx.new(|cx| TestToolWindow {
                focus: cx.focus_handle(),
            });
            w.register_tool_window(
                ToolWindowSpec {
                    id: ToolWindow(id),
                    area,
                    order: 5,
                    icon: ui::icons::IconName::TOOL_TERMINAL,
                    title: "Test",
                    command: None,
                    registered,
                },
                Some(ToolWindowContent::new(view.clone())),
                cx,
            );
            view
        })
        .unwrap();
    draw(cx, window);
    view
}

fn has_focus(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    view: &gpui::Entity<TestToolWindow>,
) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        view.read(cx).focus.contains_focused(window, cx)
    })
    .unwrap()
}

/// A tool window of the bottom area replaces Run there and back; the
/// area keeps its height, the left area its width.
#[gpui::test]
fn a_bottom_tool_window_replaces_run_and_back(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    let view = register(cx, window, "test-bottom", Area::Bottom, true);
    assert!(bounds(cx, window, "stripe-test-bottom").is_some());
    invoke(cx, window, "workbench.toggle-run", json!(null));
    // A taller bottom area.
    let splitter = bounds(cx, window, "bottom-splitter").unwrap().center();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(splitter, None, Modifiers::none());
    vcx.simulate_mouse_down(splitter, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear());
    let to = point(splitter.x, splitter.y - px(60.0));
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    draw(cx, window);
    let run = bounds(cx, window, "run-tool-window").unwrap();
    let width =
        invoke(cx, window, "workbench.state", json!(null))["tool_windows"]["project"]["width"]
            .clone();

    invoke(
        cx,
        window,
        "workbench.activate-tool-window",
        json!({"id": "test-bottom"}),
    );
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(false));
    assert_eq!(state["tool_windows"]["test-bottom"]["visible"], json!(true));
    assert_eq!(
        state["tool_windows"]["test-bottom"]["area"],
        json!("bottom")
    );
    assert_eq!(
        state["tool_window_areas"]["bottom"]["visible"],
        json!("test-bottom")
    );
    assert!(has_focus(cx, window, &view));
    let test = bounds(cx, window, "test-bottom-tool-window").unwrap();
    assert!((test.size.height - run.size.height).abs() < px(1.0));
    assert_eq!(test.bottom(), run.bottom());
    assert_eq!(state["tool_windows"]["project"]["width"], width);

    // ⌘4 brings Run back at the same height.
    invoke(cx, window, "workbench.toggle-run", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["run"]["visible"], json!(true));
    assert_eq!(
        state["tool_windows"]["test-bottom"]["visible"],
        json!(false)
    );
    assert_eq!(focused(cx, window), Some(ToolWindow::Run));
    let back = bounds(cx, window, "run-tool-window").unwrap();
    assert!((back.size.height - run.size.height).abs() < px(1.0));
    // The project tool window stayed.
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));
}

/// A tool window of the left area replaces Project (as Commit does); ⌘1
/// brings Project back at its width.
#[gpui::test]
fn a_left_tool_window_replaces_project_and_back(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    let project = bounds(cx, window, "project-tool-window").unwrap();
    let view = register(cx, window, "test-left", Area::Left, true);
    let button = bounds(cx, window, "stripe-test-left").unwrap().center();
    VisualTestContext::from_window(window.into(), cx).simulate_click(button, Modifiers::none());
    draw(cx, window);
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(false));
    assert_eq!(state["tool_windows"]["test-left"]["visible"], json!(true));
    assert!(has_focus(cx, window, &view));
    let test = bounds(cx, window, "test-left-tool-window").unwrap();
    assert_eq!(test.left(), project.left());
    assert!((test.size.width - project.size.width).abs() < px(1.0));

    // Hide Active Tool Window hides it and returns the focus to the editor.
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["test-left"]["visible"], json!(false));
    assert_eq!(state["tool_window_areas"]["left"]["visible"], json!(null));
    assert!(!has_focus(cx, window, &view));
    assert_eq!(focused(cx, window), None);

    invoke(cx, window, "workbench.toggle-project", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));
    assert_eq!(focused(cx, window), Some(ToolWindow::Project));
    let back = bounds(cx, window, "project-tool-window").unwrap();
    assert!((back.size.width - project.size.width).abs() < px(1.0));
}

/// Toggling by id: the first command shows and focuses the tool window,
/// the second one (while it has the focus) hides it. A tool window that
/// is created by its first use has no stripe button before it.
#[gpui::test]
fn toggle_tool_window_by_id(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    let view = register(cx, window, "test-late", Area::Bottom, false);
    assert!(bounds(cx, window, "stripe-test-late").is_none());
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(
        state["tool_windows"]["test-late"]["registered"],
        json!(false)
    );

    invoke(
        cx,
        window,
        "workbench.toggle-tool-window",
        json!({"id": "test-late"}),
    );
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(
        state["tool_windows"]["test-late"]["registered"],
        json!(true)
    );
    assert_eq!(state["tool_windows"]["test-late"]["visible"], json!(true));
    assert!(has_focus(cx, window, &view));
    assert!(bounds(cx, window, "stripe-test-late").is_some());

    invoke(
        cx,
        window,
        "workbench.toggle-tool-window",
        json!({"id": "test-late"}),
    );
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["test-late"]["visible"], json!(false));
    assert!(!has_focus(cx, window, &view));

    // Unknown ids and stripe buttons without a tool window change nothing.
    let before = invoke(cx, window, "workbench.state", json!(null))["tool_window_areas"].clone();
    invoke(
        cx,
        window,
        "workbench.activate-tool-window",
        json!({"id": "no-such-window"}),
    );
    invoke(
        cx,
        window,
        "workbench.activate-tool-window",
        json!({"id": "terminal"}),
    );
    let after = invoke(cx, window, "workbench.state", json!(null))["tool_window_areas"].clone();
    assert_eq!(before, after);
}

/// A tool window of the right area sits between the editor and the right
/// stripe; dragging its splitter to the left widens it.
#[gpui::test]
fn the_right_area_is_resized_by_its_splitter(cx: &mut TestAppContext) {
    let window = open(cx, test_product());
    register(cx, window, "test-right", Area::Right, true);
    invoke(
        cx,
        window,
        "workbench.activate-tool-window",
        json!({"id": "test-right"}),
    );
    let before = bounds(cx, window, "test-right-tool-window").unwrap();
    let editor = bounds(cx, window, "editor-area").unwrap();
    let stripe = bounds(cx, window, "right-stripe").unwrap();
    assert!(before.left() > editor.right());
    assert!(before.right() <= stripe.left());
    let from = bounds(cx, window, "right-splitter").unwrap().center();
    let to = point(from.x - px(50.0), from.y);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(from, None, Modifiers::none());
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    draw(cx, window);
    let after = bounds(cx, window, "test-right-tool-window").unwrap();
    assert!(
        (after.size.width - before.size.width - px(50.0)).abs() < px(1.0),
        "{before:?} -> {after:?}"
    );
    assert_eq!(after.right(), before.right());
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));
}
