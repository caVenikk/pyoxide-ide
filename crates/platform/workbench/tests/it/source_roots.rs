//! Packages, modules and source roots in the project tree (ADR 0013,
//! tests/scenarios/editor/source-roots.md) with a product that has the
//! Python product's rules: icons and hints of the rows, *Mark Directory As*
//! with real mouse events, the settings file, the one-time import of the
//! reference IDE's source folders, and the source roots of Go to File.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AppContext as _, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, WindowHandle,
};
use serde_json::{Value, json};
use ui::icons::IconName;
use workbench::{AutoExclusions, NavigationHooks, Product, RootKind, TreeHooks, Workbench};

/// The module icon of the test product (a stand-in for Python's).
const MODULE_ICON: IconName = IconName::SYMLINK;

/// `src` is a source root of its own, as the Python product detects it.
fn detect(root: &Path) -> Vec<PathBuf> {
    let src = root.join("src");
    if src.is_dir() { vec![src] } else { Vec::new() }
}

fn product() -> Product {
    Product {
        name: "Test IDE",
        auto_excluded: AutoExclusions {
            dir_names: &[".venv"],
            root_dir_markers: &[],
        },
        navigation: NavigationHooks {
            source_roots: Some(detect),
            file_location: Some(|name, folder| {
                name.ends_with(".py").then(|| folder.replace('/', "."))
            }),
            package_markers: &["__init__.py"],
        },
        tree: TreeHooks {
            root_kinds: vec![
                RootKind {
                    id: "sources",
                    name: "Sources Root",
                    icon: IconName::SOURCE_ROOT,
                    inner: true,
                    after_excluded: false,
                },
                RootKind {
                    id: "tests",
                    name: "Test Sources Root",
                    icon: IconName::TEST_ROOT,
                    inner: false,
                    after_excluded: true,
                },
            ],
            module_markers: &["pyproject.toml"],
            module_icon: Some(MODULE_ICON),
        },
        ..Product::default()
    }
}

fn open(cx: &mut TestAppContext, root: &Path) -> WindowHandle<Workbench> {
    let product = product();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    window
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

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn tree(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    let s = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(s["busy"], json!(false), "background work did not finish");
    s["tree"].clone()
}

fn row(tree: &Value, path: &str) -> Value {
    tree["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == json!(path))
        .unwrap_or_else(|| panic!("no row {path}"))
        .clone()
}

/// The icon, root kind, folder kind, bold name and hint of a row.
fn look(tree: &Value, path: &str) -> Value {
    let r = row(tree, path);
    json!([
        r["icon"],
        r["roots"]["root_kind"],
        r["roots"]["folder"],
        r["roots"]["bold"],
        r["roots"]["hint"]
    ])
}

/// Labels of the rows of each open menu level.
fn menu(tree: &Value) -> Vec<Vec<String>> {
    tree["context_menu"]["levels"]
        .as_array()
        .map(|levels| {
            levels
                .iter()
                .map(|l| {
                    l["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| i["label"].as_str().unwrap_or("-").to_owned())
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn centre(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Point<gpui::Pixels> {
    draw(cx, window);
    let name: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn right_click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    draw(cx, window);
}

fn hover(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    VisualTestContext::from_window(window.into(), cx).simulate_mouse_move(
        at,
        None,
        Modifiers::none(),
    );
    draw(cx, window);
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    draw(cx, window);
}

/// The *Mark Directory As* submenu of a row, opened with the mouse.
fn mark_menu(cx: &mut TestAppContext, window: WindowHandle<Workbench>, path: &str) -> Vec<String> {
    right_click(cx, window, &format!("tree-row:{path}"));
    hover(cx, window, "menu-item:Mark Directory As");
    let levels = menu(&tree(cx, window));
    assert_eq!(levels.len(), 2, "{levels:?}");
    levels[1].clone()
}

fn escape(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.simulate_keystrokes(window.into(), "escape escape");
    draw(cx, window);
}

fn layout(root: &Path) {
    for (path, text) in [
        ("pyproject.toml", "[project]\nname = \"demo\"\n"),
        ("app/__init__.py", ""),
        ("app/api/__init__.py", ""),
        ("app/api/views.py", "v = 1\n"),
        ("app/static/site.css", "a {}\n"),
        ("services/api/pyproject.toml", "[project]\nname = \"api\"\n"),
        ("services/api/main.py", "m = 1\n"),
        ("src/lib/core.py", "c = 1\n"),
        ("tests/test_x.py", "t = 1\n"),
        (".venv/lib/site.py", "s = 1\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn settings(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(".oxide/project.toml")).ok()
}

/// The project root's location as the tree shows it.
fn location(root: &Path) -> String {
    project::Project::new(root).display_root()
}

#[gpui::test]
fn packages_modules_and_roots_in_the_tree(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    for path in ["app", "services"] {
        invoke(cx, window, "project.tree.reveal", json!({"path": path}));
        invoke(cx, window, "project.tree.expand", json!(null));
    }
    let t = tree(cx, window);
    let loc = location(&root);
    // The project root: a module (it has pyproject.toml) and no root yet.
    assert_eq!(
        look(&t, ""),
        json!([MODULE_ICON.0, null, "module", true, loc])
    );
    // Packages, a nested module, the detected source root, plain folders.
    assert_eq!(
        look(&t, "app"),
        json!(["nodes/package", null, "package", false, null])
    );
    assert_eq!(
        look(&t, "app/api"),
        json!(["nodes/package", null, "package", false, null])
    );
    assert_eq!(
        look(&t, "app/static"),
        json!(["nodes/folder", null, null, false, null])
    );
    assert_eq!(
        look(&t, "services"),
        json!(["nodes/folder", null, null, false, null])
    );
    assert_eq!(
        look(&t, "services/api"),
        json!([MODULE_ICON.0, null, "module", true, null])
    );
    assert_eq!(
        look(&t, "src"),
        json!(["nodes/sourceRoot", "sources", null, false, null])
    );
    assert_eq!(
        look(&t, "tests"),
        json!(["nodes/folder", null, null, false, null])
    );
    assert_eq!(
        look(&t, ".venv"),
        json!(["nodes/excludeRoot", null, null, false, null])
    );
    assert_eq!(t["roots"], json!({"sources": ["src"]}));
    assert!(settings(&root).is_none(), "detected roots are not stored");

    // The project root offers both kinds, and nothing else to mark.
    assert_eq!(
        mark_menu(cx, window, ""),
        ["Sources Root", "Test Sources Root"]
    );
    click(cx, window, "menu-item:Sources Root");
    let t = tree(cx, window);
    assert_eq!(t["context_menu"], json!(null), "the menu closed");
    assert_eq!(
        row(&t, "")["roots"]["hint"],
        json!(format!("sources root, \u{2009}{loc}"))
    );
    assert_eq!(row(&t, "")["icon"], json!(MODULE_ICON.0));
    // The detected root is stored with the marked one.
    assert_eq!(t["roots"], json!({"sources": [".", "src"]}));
    let text = settings(&root).expect("the settings were written");
    assert!(
        text.contains("[roots]\nsources = [\n    \".\",\n    \"src\",\n]\n"),
        "{text}"
    );

    // Inside a source root a test sources root cannot be marked.
    assert_eq!(mark_menu(cx, window, "tests"), ["Sources Root", "Excluded"]);
    escape(cx, window);
    // A root offers the other kind and Unmark; Excluded sits between.
    assert_eq!(
        mark_menu(cx, window, "src"),
        ["Excluded", "Test Sources Root", "Unmark as Sources Root"]
    );
    click(cx, window, "menu-item:Test Sources Root");
    let t = tree(cx, window);
    assert_eq!(
        look(&t, "src"),
        json!(["nodes/testRoot", "tests", null, false, null])
    );
    assert_eq!(t["roots"], json!({"sources": ["."], "tests": ["src"]}));

    // A nested module that is a sources root says so after its name.
    assert_eq!(
        mark_menu(cx, window, "services/api"),
        ["Sources Root", "Excluded"]
    );
    click(cx, window, "menu-item:Sources Root");
    let t = tree(cx, window);
    assert_eq!(
        look(&t, "services/api"),
        json!([MODULE_ICON.0, "sources", "module", true, "sources root"])
    );
    // A package that is a root shows the root.
    invoke(
        cx,
        window,
        "project.tree.mark-root",
        json!({"kind": "sources", "path": "app/api"}),
    );
    let t = tree(cx, window);
    assert_eq!(
        look(&t, "app/api"),
        json!(["nodes/sourceRoot", "sources", "package", false, null])
    );

    // Unmarking by command; unknown kinds, files and paths outside the
    // project change nothing.
    invoke(cx, window, "project.tree.unmark-root", json!({"path": "."}));
    invoke(
        cx,
        window,
        "project.tree.unmark-root",
        json!({"path": "app/api"}),
    );
    for args in [
        json!({"kind": "resources", "path": "app"}),
        json!({"kind": "sources", "path": "app/api/views.py"}),
        json!({"kind": "sources", "path": "/elsewhere"}),
        json!({"kind": "sources", "path": ".venv"}),
    ] {
        invoke(cx, window, "project.tree.mark-root", args);
    }
    let t = tree(cx, window);
    assert_eq!(row(&t, "")["roots"]["hint"], json!(loc));
    assert_eq!(
        t["roots"],
        json!({"sources": ["services/api"], "tests": ["src"]})
    );

    // An excluded folder offers only Cancel Exclusion; a folder inside it
    // the kinds but not Excluded.
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": "app"}),
    );
    assert_eq!(mark_menu(cx, window, "app"), ["Cancel Exclusion"]);
    escape(cx, window);
    let t = tree(cx, window);
    assert_eq!(
        look(&t, "app/api"),
        json!(["nodes/excludeRoot", null, "package", false, null])
    );
    assert_eq!(
        mark_menu(cx, window, "app/api"),
        ["Sources Root", "Test Sources Root"]
    );
    escape(cx, window);
    // A folder a product rule excludes has nothing to mark.
    right_click(cx, window, "tree-row:.venv");
    let levels = menu(&tree(cx, window));
    assert!(
        !levels[0].iter().any(|l| l == "Mark Directory As"),
        "{levels:?}"
    );
    escape(cx, window);

    // Reopening keeps the stored roots; the detection no longer applies.
    invoke(
        cx,
        window,
        "project.tree.unmark-root",
        json!({"path": "src"}),
    );
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    let t = tree(cx, window);
    assert_eq!(t["roots"], json!({"sources": ["services/api"]}));
    assert_eq!(
        look(&t, "src"),
        json!(["nodes/folder", null, null, false, null])
    );
}

#[gpui::test]
fn roots_follow_renames_and_packages_their_marker_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    invoke(
        cx,
        window,
        "project.tree.mark-root",
        json!({"kind": "tests", "path": "tests"}),
    );
    invoke(
        cx,
        window,
        "project.tree.rename",
        json!({"path": "tests", "name": "checks"}),
    );
    let t = tree(cx, window);
    assert_eq!(t["roots"], json!({"sources": ["src"], "tests": ["checks"]}));
    assert_eq!(
        look(&t, "checks"),
        json!(["nodes/testRoot", "tests", null, false, null])
    );
    assert!(settings(&root).unwrap().contains("\"checks\""));

    // A new `__init__.py` makes its folder a package at once.
    invoke(
        cx,
        window,
        "project.tree.new-file",
        json!({"name": "__init__.py", "directory": "services"}),
    );
    assert_eq!(
        row(&tree(cx, window), "services")["icon"],
        json!("nodes/package")
    );
    // So does one created outside the application, once the folder is
    // looked at again.
    std::fs::write(root.join("checks/__init__.py"), "").unwrap();
    std::fs::remove_file(root.join("app/__init__.py")).unwrap();
    invoke(
        cx,
        window,
        "project.tree.reload-from-disk",
        json!({"paths": ["checks", "app"]}),
    );
    let t = tree(cx, window);
    assert_eq!(
        look(&t, "app"),
        json!(["nodes/folder", null, null, false, null])
    );
    // A root keeps its root icon.
    assert_eq!(
        look(&t, "checks"),
        json!(["nodes/testRoot", "tests", "package", false, null])
    );
}

#[gpui::test]
fn source_folders_of_the_reference_are_imported_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    layout(&root);
    // A project Pyoxide opened before (its exclusions are stored) and the
    // reference IDE marks folders in.
    std::fs::create_dir_all(root.join(".oxide")).unwrap();
    std::fs::write(
        root.join(".oxide/project.toml"),
        "version = 1\n\n[exclusions]\npaths = [\"app/static\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join(".idea")).unwrap();
    std::fs::write(
        root.join(".idea/demo.iml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<module type="PYTHON_MODULE" version="4">
  <component name="NewModuleRootManager">
    <content url="file://$MODULE_DIR$">
      <sourceFolder url="file://$MODULE_DIR$" isTestSource="false" />
      <sourceFolder url="file://$MODULE_DIR$/services/api" isTestSource="false" />
      <sourceFolder url="file://$MODULE_DIR$/tests" isTestSource="true" />
      <excludeFolder url="file://$MODULE_DIR$/app" />
    </content>
  </component>
</module>"#,
    )
    .unwrap();
    let window = open(cx, &root);
    invoke(
        cx,
        window,
        "project.tree.reveal",
        json!({"path": "services/api"}),
    );
    let t = tree(cx, window);
    let loc = location(&root);
    assert_eq!(
        look(&t, ""),
        json!([
            MODULE_ICON.0,
            "sources",
            "module",
            true,
            format!("sources root, \u{2009}{loc}")
        ])
    );
    assert_eq!(
        look(&t, "services/api"),
        json!([MODULE_ICON.0, "sources", "module", true, "sources root"])
    );
    assert_eq!(
        look(&t, "tests"),
        json!(["nodes/testRoot", "tests", null, false, null])
    );
    assert_eq!(
        t["roots"],
        json!({"sources": [".", "services/api", "src"], "tests": ["tests"]})
    );
    // The exclusions stay Pyoxide's: `.idea`'s are not read again.
    assert_eq!(t["exclusions"], json!(["app/static"]));
    let text = settings(&root).unwrap();
    assert!(text.contains("[roots]"), "{text}");
    assert!(text.contains("paths = [\"app/static\"]"), "{text}");

    // Go to File names a module from its source root.
    let panel = window.read_with(cx, |w, _| w.panel().clone()).unwrap();
    let roots = panel.read_with(cx, |p, _| p.source_roots());
    assert_eq!(
        roots,
        [
            root.clone(),
            root.join("services/api"),
            root.join("src"),
            root.join("tests")
        ]
    );
    let files = window
        .read_with(cx, |w, cx| w.project_files().read(cx).source_roots())
        .unwrap();
    assert_eq!(*files, roots);
}
