//! `pyoxide-ide`: the Pyoxide Python IDE. Composes the platform workbench
//! with what is specific to Python (file type icons, tool windows, the
//! interpreter widget).

use std::sync::Arc;

use editor::SyntaxHighlighter;
use ui::icons::{IconName, IconSvg};
use workbench::{FileType, Product, StripeItem};

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
    }
}

fn product() -> Product {
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
        interpreter_label: Some("No interpreter"),
    }
}

fn main() {
    let paths = std::env::args_os().skip(1).map(Into::into).collect();
    workbench::run(product(), paths);
}

#[cfg(test)]
mod tests {
    use super::*;

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
