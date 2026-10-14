//! `pyoxide-ide`: the Pyoxide Python IDE. Composes the platform workbench
//! with what is specific to Python (file type icons, tool windows, the
//! interpreter widget, running files).

pub mod interpreter;
pub mod run;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::SyntaxHighlighter;
use ui::icons::{IconName, IconSvg};
use workbench::{AutoExclusions, FileType, NavigationHooks, Product, RunHook, StripeItem};

/// Icons of the Python product (IntelliJ Platform Community, Apache-2.0,
/// see NOTICE).
const ICONS: &[IconSvg] = &[
    IconSvg {
        name: "python/file",
        light: include_bytes!("../assets/icons/python.svg"),
        dark: Some(include_bytes!("../assets/icons/python_dark.svg")),
    },
    IconSvg {
        name: "python/packages",
        light: include_bytes!("../assets/icons/pythonPackages@20x20.svg"),
        dark: Some(include_bytes!(
            "../assets/icons/pythonPackages@20x20_dark.svg"
        )),
    },
    IconSvg {
        name: "python/console",
        light: include_bytes!("../assets/icons/pythonConsoleToolWindow@20x20.svg"),
        dark: Some(include_bytes!(
            "../assets/icons/pythonConsoleToolWindow@20x20_dark.svg"
        )),
    },
];

/// Python source files.
fn python() -> FileType {
    FileType {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        icon: IconName("python/file"),
        // A block opens after a colon at the end of a line and ends after
        // a statement that leaves it.
        indent_after: &[':'],
        dedent_after: &["return", "pass", "break", "continue", "raise"],
        line_comment: Some("# "),
        quotes: &['"', '\''],
        triple_quotes: true,
        // String prefixes: raw, bytes, formatted, template, unicode.
        string_prefixes: &['r', 'R', 'b', 'B', 'f', 'F', 't', 'T', 'u', 'U'],
        highlighter: Some(Arc::new(SyntaxHighlighter::factory(py_syntax::language()))),
        language_id: Some("python"),
    }
}

/// Virtual environments are excluded from a project on their own, as the
/// reference IDE does: every folder named `.venv`, and (standing in for the
/// environment of the project's interpreter until interpreters are chosen
/// in the IDE) a folder under the project root that holds a `pyvenv.cfg`.
const EXCLUDED: AutoExclusions = AutoExclusions {
    dir_names: &[".venv"],
    root_dir_markers: &["pyvenv.cfg"],
};

/// Source roots as the reference IDE marks them when a folder opens as a
/// project: the root package folder that `setup.py` names
/// (`package_dir={"": "lib"}`), and the folder `src`.
fn source_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("setup.py"))
        && let Some(dir) = root_package_dir(&text)
    {
        let dir = root.join(dir);
        if dir.is_dir() {
            roots.push(dir);
        }
    }
    let src = root.join("src");
    if src.is_dir() && !roots.contains(&src) {
        roots.push(src);
    }
    roots
}

/// The folder of the root package in a `setup(...)` call: the value of the
/// empty key of its `package_dir` dictionary.
fn root_package_dir(setup_py: &str) -> Option<String> {
    let at = setup_py.find("package_dir")?;
    let rest = &setup_py[at + "package_dir".len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let body = rest.strip_prefix('{')?;
    let body = &body[..body.find('}')?];
    let strings = |text: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '"' || c == '\'' {
                out.push(chars.by_ref().take_while(|&x| x != c).collect());
            }
        }
        out
    };
    body.split(',').find_map(|pair| {
        let (key, value) = pair.split_once(':')?;
        let key = strings(key);
        let value = strings(value);
        (key.len() == 1 && key[0].is_empty() && value.len() == 1).then(|| value[0].clone())
    })
}

/// A Python file's location in Go to File: its package, the folder from
/// its source root with dots (`flask.sansio`).
fn file_location(name: &str, folder: &str) -> Option<String> {
    let python = [".py", ".pyi", ".pyw"].iter().any(|e| name.ends_with(e));
    python.then(|| folder.replace('/', "."))
}

/// Registers the product's commands, globals and icons (called by the
/// workbench at the end of its `init`).
fn init(cx: &mut gpui::App) {
    interpreter::init(cx);
}

/// The Pyoxide product for the workbench.
pub fn product() -> Product {
    Product {
        name: "Pyoxide",
        file_types: vec![python()],
        icons: ICONS,
        left_stripe_bottom: vec![
            StripeItem {
                id: "stripe-python-packages",
                icon: IconName("python/packages"),
                title: "Python Packages",
            },
            StripeItem {
                id: "stripe-python-console",
                icon: IconName("python/console"),
                title: "Python Console",
            },
        ],
        status_widgets: vec![interpreter::status_widget()],
        // The reference IDE's masks for Python: the language's files, then
        // markup and styles, then notebooks, stubs and Cython.
        find_file_masks: vec![
            "*.py", "*.xml", "*.html", "*.css", "*.ipynb", "*.pyi", "*.pyx", "*.pxd", "*.pxi",
        ],
        auto_excluded: EXCLUDED,
        navigation: NavigationHooks {
            source_roots: Some(source_roots),
            file_location: Some(file_location),
            package_markers: &["__init__.py"],
        },
        init: Some(init),
        run: Some(RunHook::new(run::PythonRunProvider)),
        // Language services come with the Python services package (#87).
        code_intel: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_roots_as_the_reference_marks_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(source_roots(root).is_empty());
        std::fs::create_dir(root.join("src")).unwrap();
        assert_eq!(source_roots(root), [root.join("src")]);
        std::fs::create_dir(root.join("lib")).unwrap();
        std::fs::write(
            root.join("setup.py"),
            "setup(\n    name='x',\n    package_dir={'': \"lib\"},\n)\n",
        )
        .unwrap();
        assert_eq!(source_roots(root), [root.join("lib"), root.join("src")]);
        assert_eq!(root_package_dir("setup(package_dir = {'x': 'y'})"), None);
        assert_eq!(
            file_location("app.py", "flask/sansio").as_deref(),
            Some("flask.sansio")
        );
        assert_eq!(file_location("app.rst", "docs"), None);
    }

    #[test]
    fn product_icons_rasterize() {
        for icon in ICONS {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            assert!(
                ui::icons::rasterize(icon.dark.unwrap(), 32).is_some(),
                "{}",
                icon.name
            );
        }
        assert_eq!(product().left_stripe_bottom.len(), 2);
    }

    /// Python files are highlighted off the UI thread, and highlighting
    /// follows edits; driven through command ids only.
    #[gpui::test]
    fn python_files_are_highlighted_and_follow_edits(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use serde_json::{Value, json};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.py"), "def main():\n    return 1\n").unwrap();
        let product = std::rc::Rc::new(product());
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let invoke = |cx: &mut gpui::TestAppContext, id: &str, args: Value| -> Value {
            let out = cx
                .update_window(window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .unwrap()
                .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            cx.run_until_parked();
            out
        };
        invoke(
            cx,
            "project.open-folder",
            json!({"path": dir.path().display().to_string()}),
        );
        invoke(cx, "project.open-file", json!({"path": "app.py"}));
        let line0 = invoke(cx, "editor.highlights", json!({"line": 0}));
        assert_eq!(line0["current"], json!(true));
        assert_eq!(
            line0["spans"],
            json!([
                {"start": 0, "end": 3, "role": "keyword"},
                {"start": 4, "end": 8, "role": "function-decl"},
            ])
        );
        // Typing a decorator above re-parses incrementally.
        invoke(cx, "editor.type", json!({"text": "@staticmethod\n"}));
        let line0 = invoke(cx, "editor.highlights", json!({"line": 0}));
        assert_eq!(line0["current"], json!(true));
        assert_eq!(line0["spans"][0]["role"], json!("decorator"));
        let line1 = invoke(cx, "editor.highlights", json!({"line": 1}));
        assert_eq!(
            line1["spans"][0],
            json!({"start": 0, "end": 3, "role": "keyword"})
        );
    }

    /// Typing an incomplete line above valid code keeps the code below
    /// highlighted (the lexical layer, ADR 0007).
    #[gpui::test]
    fn highlighting_survives_a_syntax_error_while_typing(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use serde_json::{Value, json};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("app.py"),
            "def func_0(a, b=None):\n    return a\n",
        )
        .unwrap();
        let product = std::rc::Rc::new(product());
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let invoke = |cx: &mut gpui::TestAppContext, id: &str, args: Value| -> Value {
            let out = cx
                .update_window(window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .unwrap()
                .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            cx.run_until_parked();
            out
        };
        invoke(
            cx,
            "project.open-folder",
            json!({"path": dir.path().display().to_string()}),
        );
        invoke(cx, "project.open-file", json!({"path": "app.py"}));
        let expected = json!([
            {"start": 0, "end": 3, "role": "keyword"},
            {"start": 4, "end": 10, "role": "function-decl"},
        ]);
        let line0 = invoke(cx, "editor.highlights", json!({"line": 0}));
        assert_eq!(
            line0["spans"].as_array().unwrap()[..2],
            expected.as_array().unwrap()[..]
        );
        for text in ["def fib\n", "def fib(\n"] {
            invoke(
                cx,
                "editor.set-selection",
                json!({"anchor": {"line": 0, "column": 0}, "head": {"line": 0, "column": 0}}),
            );
            invoke(cx, "editor.type", json!({"text": text}));
            let after = invoke(cx, "editor.highlights", json!({"line": 1}));
            assert_eq!(after["current"], json!(true));
            assert_eq!(
                after["spans"].as_array().unwrap()[..2],
                expected.as_array().unwrap()[..],
                "after typing {text:?}"
            );
            invoke(cx, "editor.undo", json!(null));
        }
    }

    /// Python editing through real keystrokes of the platform keymap:
    /// Extend Selection follows the syntax tree, Shrink Selection goes back,
    /// quotes pair, and the line comment is `# `.
    #[gpui::test]
    fn python_editing_through_keystrokes(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use serde_json::{Value, json};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.py"), "x = foo(bar, \"baz qux\")\n").unwrap();
        let product = std::rc::Rc::new(product());
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let draw = |cx: &mut gpui::TestAppContext| {
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        };
        let invoke = |cx: &mut gpui::TestAppContext, id: &str, args: Value| {
            cx.update_window(window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            draw(cx);
        };
        invoke(
            cx,
            "project.open-folder",
            json!({"path": dir.path().display().to_string()}),
        );
        invoke(cx, "project.open-file", json!({"path": "app.py"}));
        let editor = window
            .read_with(cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .unwrap();
        let selected = |cx: &mut gpui::TestAppContext| {
            editor.read_with(cx, |e, _| {
                e.buffer().slice(e.selections().primary().range())
            })
        };
        let (extend, shrink, comment) = if cfg!(target_os = "macos") {
            ("alt-up", "alt-down", "cmd-/")
        } else {
            ("ctrl-w", "ctrl-shift-w", "ctrl-/")
        };
        invoke(
            cx,
            "editor.set-selection",
            json!({"anchor": {"line": 0, "column": 20}, "head": {"line": 0, "column": 20}}),
        );
        for step in [
            "qux",
            "baz qux",
            "\"baz qux\"",
            "(bar, \"baz qux\")",
            "foo(bar, \"baz qux\")",
            "x = foo(bar, \"baz qux\")",
        ] {
            cx.simulate_keystrokes(window.into(), extend);
            draw(cx);
            assert_eq!(selected(cx), step);
        }
        cx.simulate_keystrokes(window.into(), shrink);
        draw(cx);
        assert_eq!(selected(cx), "foo(bar, \"baz qux\")");

        // Typing at the end of the line: a quote pairs, a bracket closes.
        cx.simulate_keystrokes(window.into(), "end");
        cx.simulate_input(window.into(), " + str(\"a");
        draw(cx);
        let text = editor.read_with(cx, |e, _| e.text());
        assert_eq!(text, "x = foo(bar, \"baz qux\") + str(\"a\")\n");
        cx.simulate_keystrokes(window.into(), comment);
        draw(cx);
        let text = editor.read_with(cx, |e, _| e.text());
        assert_eq!(text, "# x = foo(bar, \"baz qux\") + str(\"a\")\n");
    }
}
