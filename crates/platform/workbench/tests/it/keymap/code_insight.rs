//! Documentation, problems, fixes and navigation: the bindings of `keymaps/*/code-insight.toml`.

use super::*;

area_tests!("code-insight");

/// A warning on `os` of `import os` in the active editor.
fn mark_os(h: &mut H) {
    let editor = h.editor();
    let mark = ::editor::DiagnosticMark {
        range: 7..9,
        severity: ::editor::MarkSeverity::Warning,
        unnecessary: true,
        deprecated: false,
        messages: vec![::editor::MarkMessage {
            severity: ::editor::MarkSeverity::Warning,
            text: "`os` imported but unused".into(),
            source: Some("Ruff".into()),
            code: None,
            href: None,
        }],
    };
    h.cx.update(|cx| editor.update(cx, |e, cx| e.set_diagnostics(vec![mark], cx)));
    h.draw();
}

fn check_docs(h: &mut H, command: &str, keys: &str, what: &str) {
    let state = |h: &mut H| h.invoke("code.state", json!(null));
    match command {
        "code.quick-doc" => {
            h.set_editor("x = al|pha");
            h.press(keys);
            assert_eq!(state(h)["popup"]["kind"], "quick-doc", "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "documentation.close" => {
            h.set_editor("x = al|pha");
            h.invoke("code.quick-doc", json!(null));
            assert!(!state(h)["popup"].is_null(), "{what}");
            h.press(keys);
            assert!(state(h)["popup"].is_null(), "{what}");
            assert_eq!(h.editor_marked(), "x = al|pha", "{what}");
        }
        "code.parameter-info" => {
            h.set_editor("f(a, |b)");
            h.press(keys);
            assert_eq!(state(h)["signature"]["current"], "b", "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "parameter-info.close" => {
            h.set_editor("f(a, |b)");
            h.invoke("code.parameter-info", json!(null));
            assert_eq!(state(h)["signature"]["shown"], true, "{what}");
            h.press(keys);
            assert!(state(h)["signature"].is_null(), "{what}");
            assert_eq!(h.editor_marked(), "f(a, |b)", "{what}");
        }
        "code.error-description" => {
            h.set_editor("import os|\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(state(h)["popup"]["kind"], "error-description", "{what}");
        }
        "code.apply-first-fix" => {
            h.set_editor("import os|\nx\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(h.editor_marked(), "|x\n", "{what}");
        }
        "code.show-intentions" => {
            h.set_editor("import os|\nx\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(state(h)["list"]["name"], "intentions", "{what}");
            assert_eq!(
                state(h)["list"]["selected"],
                "Remove unused import",
                "{what}"
            );
        }
        "code.go-to-declaration" => {
            h.set_editor("a|bc\nx\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "abc\n|x\n", "{what}");
        }
        "code.show-usages" => {
            h.set_editor("a|bc\nx\n");
            h.press(keys);
            assert_eq!(state(h)["list"]["name"], "usages", "{what}");
            assert_eq!(state(h)["list"]["items"][0]["text"], "x", "{what}");
        }
        "code.reformat" => {
            h.set_editor("x=1|\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = 1|\n", "{what}");
        }
        "code.optimize-imports" => {
            h.set_editor("import b|\nimport a\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "import a\nimport b\n|", "{what}");
        }
        _ => unreachable!("{command}"),
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "code.next-problem" | "code.previous-problem" => {
            // Errors before the nearer warning; forward from the caret, or
            // back around the start.
            h.set_editor("a = 1|\nb = undefined\nc = x == None");
            let editor = h.editor();
            h.cx.update(|cx| {
                editor.update(cx, |e, cx| {
                    let problem =
                        |range: std::ops::Range<usize>, severity| ::editor::DiagnosticMark {
                            range,
                            severity,
                            unnecessary: false,
                            deprecated: false,
                            messages: Vec::new(),
                        };
                    e.set_diagnostics(
                        vec![
                            problem(0..1, ::editor::MarkSeverity::Warning),
                            problem(10..19, ::editor::MarkSeverity::Error),
                            problem(24..33, ::editor::MarkSeverity::Error),
                        ],
                        cx,
                    );
                })
            });
            h.press(keys);
            let expected = if b.command == "code.next-problem" {
                "a = 1\nb = |undefined\nc = x == None"
            } else {
                "a = 1\nb = undefined\nc = |x == None"
            };
            assert_eq!(h.editor_marked(), expected, "{what}");
        }
        "code.quick-doc"
        | "documentation.close"
        | "code.parameter-info"
        | "parameter-info.close"
        | "code.error-description"
        | "code.apply-first-fix"
        | "code.show-intentions"
        | "code.go-to-declaration"
        | "code.show-usages"
        | "code.reformat"
        | "code.optimize-imports" => check_docs(h, &b.command, keys, &what),
        _ => return false,
    }
    true
}
