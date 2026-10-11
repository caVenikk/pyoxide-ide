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
        // A block opens after a colon at the end of a line.
        indent_after: &[':'],
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
}
