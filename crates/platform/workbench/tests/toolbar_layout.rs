//! The main toolbar and the status bar while the window gets narrower, with
//! a project in a deep folder (tests/scenarios/editor/
//! narrow-window-toolbar.md): nothing overlaps,
//! the texts give way first (the widest one first), then the toolbar's
//! right group runs past the right edge; the status bar cuts the
//! breadcrumbs, then its widgets run past the left edge.

use std::path::Path;
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Pixels, TestAppContext, VisualTestContext, WindowHandle, px, size,
};
use serde_json::{Value, json};
use workbench::main_toolbar::GAP;
use workbench::{FileType, Product, Workbench};

const EPS: f32 = 0.6;

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

fn invoke(cx: &mut TestAppContext, window: WindowHandle<Workbench>, id: &str, args: Value) {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke(id, args, window, cx)
    })
    .unwrap()
    .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    cx.run_until_parked();
}

/// A project in a deep folder with `src/app.py` open.
fn open_deep_project(cx: &mut TestAppContext) -> (tempfile::TempDir, WindowHandle<Workbench>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("work/clients/2026/projects/backend-services/skin-tech-api");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/app.py"), "print('hi')\n").unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": Path::new("src").join("app.py").display().to_string()}),
    );
    (dir, window)
}

struct Frame {
    width: f32,
    project: Bounds<Pixels>,
    file: Bounds<Pixels>,
    /// The run configuration selector and the run widget's buttons.
    run: Bounds<Pixels>,
    buttons: Bounds<Pixels>,
    settings: Bounds<Pixels>,
    /// Widths of the texts: project, file, run configuration.
    texts: [f32; 3],
    status_left: Bounds<Pixels>,
    status_right: Bounds<Pixels>,
}

fn frame(cx: &mut VisualTestContext, width: f32) -> Frame {
    cx.simulate_resize(size(px(width), px(800.0)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let mut b = |s: &'static str| {
        cx.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} is not drawn at {width}"))
    };
    Frame {
        width,
        project: b("project-widget"),
        file: b("file-path-widget"),
        run: b("run-configuration"),
        buttons: b("run-buttons"),
        settings: b("toolbar-settings"),
        texts: [
            f32::from(b("project-widget-label").size.width),
            f32::from(b("file-path-widget-text").size.width),
            f32::from(b("run-configuration-text").size.width),
        ],
        status_left: b("status-left"),
        status_right: b("status-right"),
    }
}

/// Widths from wide to narrow, below the window's minimum too.
fn frames(cx: &mut VisualTestContext) -> Vec<Frame> {
    (120..=2400)
        .rev()
        .step_by(9)
        .map(|w| frame(cx, w as f32))
        .collect()
}

fn x(p: Pixels) -> f32 {
    f32::from(p)
}

#[gpui::test]
fn the_toolbar_never_overlaps_and_its_texts_give_way_first(cx: &mut TestAppContext) {
    let (_dir, window) = open_deep_project(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let frames = frames(cx);
    let inset = if cfg!(target_os = "macos") {
        workbench::chrome::TRAFFIC_LIGHTS_INSET
    } else {
        8.0
    };
    let narrowest = frames.last().unwrap();
    let widest = frames.first().unwrap();
    let buttons = cx.update(|_, cx| workbench::run::widget::buttons_width(cx));
    let chrome = [
        workbench::chrome::project_widget_chrome(true),
        workbench::chrome::file_path_widget_chrome(),
        workbench::run::widget::selector_chrome(&Default::default()),
    ];
    for f in &frames {
        let w = f.width;
        // The widgets are as wide as the layout made them: their parts
        // and the text's box.
        let widgets = [f.project, f.file, f.run];
        for i in 0..3 {
            let drawn = x(widgets[i].size.width);
            assert!(
                (drawn - chrome[i] - f.texts[i]).abs() < EPS,
                "{w}: widget {i}"
            );
        }
        // The buttons follow the selector and keep their width.
        assert!((x(f.buttons.size.width) - buttons).abs() < EPS, "{w}");
        assert!((x(f.buttons.left()) - x(f.run.right())).abs() < EPS, "{w}");
        // The groups keep two gaps apart, at every width.
        assert_eq!(x(f.project.left()), inset, "{w}");
        assert!(
            x(f.project.right()) + 2.0 * GAP <= x(f.file.left()) + EPS,
            "{w}: project {:?}, file {:?}",
            f.project,
            f.file
        );
        assert!(
            x(f.file.right()) + 2.0 * GAP <= x(f.run.left()) + EPS,
            "{w}: file {:?}, run {:?}",
            f.file,
            f.run
        );
        assert!(x(f.buttons.right()) <= x(f.settings.left()), "{w}");
        // The right group stays inside the window until every text is at
        // its narrowest.
        let at_minimum = (0..3).all(|i| f.texts[i] <= narrowest.texts[i] + EPS);
        if !at_minimum {
            assert!(
                x(f.settings.right()) + 5.0 <= w + EPS,
                "{w}: the gear at {:?}",
                f.settings
            );
        }
    }
    // Wide, nothing is cut and the gear sits at the right edge.
    assert!(x(widest.settings.right()) + 5.0 >= widest.width - EPS);
    // Narrowest, the right group runs past the edge.
    assert!(x(narrowest.settings.right()) > narrowest.width);
    // Every text only shrinks, and the file's (the widest) first.
    for i in 0..3 {
        for pair in frames.windows(2) {
            assert!(
                pair[1].texts[i] <= pair[0].texts[i] + EPS,
                "text {i} grew from {} to {}",
                pair[0].width,
                pair[1].width
            );
        }
        assert!(
            narrowest.texts[i] < widest.texts[i],
            "text {i} never shrank"
        );
    }
    let first_cut = |i: usize| {
        frames
            .iter()
            .find(|f| f.texts[i] < widest.texts[i] - EPS)
            .unwrap()
            .width
    };
    assert!(
        first_cut(1) > first_cut(0),
        "the file gives way before the project"
    );
    assert!(
        first_cut(1) > first_cut(2),
        "the file gives way before the run widget"
    );
    // Once at their minimum the groups keep their places and slide out.
    let fixed: Vec<&Frame> = frames
        .iter()
        .filter(|f| x(f.settings.right()) > f.width)
        .collect();
    assert!(fixed.len() > 2);
    for f in &fixed {
        assert_eq!(f.run.left(), fixed[0].run.left(), "{}", f.width);
    }
}

#[gpui::test]
fn the_status_bar_cuts_the_breadcrumbs_then_slides_out(cx: &mut TestAppContext) {
    let (_dir, window) = open_deep_project(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let frames = frames(cx);
    let widgets = frames[0].status_right.size.width;
    for f in &frames {
        let w = f.width;
        // The widgets keep their width and the right edge.
        assert_eq!(f.status_right.size.width, widgets, "{w}");
        assert!((x(f.status_right.right()) - (w - 10.0)).abs() < EPS, "{w}");
        // The breadcrumbs end before them.
        assert!(
            x(f.status_left.right()) + 16.0 <= x(f.status_right.left()) + EPS
                || x(f.status_left.size.width) < EPS,
            "{w}: {:?} {:?}",
            f.status_left,
            f.status_right
        );
        // Too narrow for the widgets, they run past the left edge.
        if x(widgets) + 10.0 > w + EPS {
            assert!(x(f.status_right.left()) < 0.0, "{w}");
        }
    }
    // The breadcrumbs shrink to nothing, the widgets slide out.
    let narrowest = frames.last().unwrap();
    assert!(x(frames[0].status_left.size.width) > 0.0);
    assert!(x(narrowest.status_left.size.width) < EPS);
    assert!(x(narrowest.status_right.left()) < 0.0);
}
