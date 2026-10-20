//! Running and stopping, and the Run tool window: the bindings of `keymaps/*/run.toml`.

use super::*;

area_tests!("run");

impl H<'_> {
    /// The Run console, or the Run tool window itself, gets the focus.
    pub(super) fn focus_run(&mut self, console: bool) {
        let run = self.workbench(|w, _| w.run_window().clone());
        let handle = self.cx.update(|cx| {
            if console {
                run.read(cx).content_focus(cx)
            } else {
                run.focus_handle(cx)
            }
        });
        self.focus(handle);
    }

    fn run_focused(&mut self) -> bool {
        let run = self.workbench(|w, _| w.run_window().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                run.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }
}

/// The keys that the Run tool window and its console (`console`) share:
/// Escape, Rerun and Close Tab.
pub(super) fn check_tab_key(h: &mut H, command: &str, keys: &str, what: &str, console: bool) {
    match command {
        "workbench.focus-editor" => {
            h.start_run(&support::script("echo done", "echo done"), "");
            h.focus_run(console);
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
        }
        "run.rerun" => {
            let first = h.start_run(&support::long_running(), "started");
            h.focus_run(console);
            h.press(keys);
            h.wait_run("the rerun", |s| {
                s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
            });
            assert_eq!(h.run()["tabs"].as_array().unwrap().len(), 1, "{what}");
        }
        "run.close-tab" => {
            h.start_run(&support::script("echo done", "echo done"), "");
            h.focus_run(console);
            h.press(keys);
            assert!(h.run()["tabs"].as_array().unwrap().is_empty(), "{what}");
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
    // The Run tool window and its console.
    let run_context = b.context.as_deref().is_some_and(|c| c.starts_with("Run"));
    let console_context = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("RunConsole"));
    match b.command.as_str() {
        "workbench.toggle-run" => {
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["run"]["visible"], json!(true), "{what}");
            assert!(
                h.run_focused(),
                "{what}: first press focuses the Run tool window"
            );
            assert!(!h.editor_focused(), "{what}");
            h.press(keys);
            let s = h.state();
            assert_eq!(
                s["tool_windows"]["run"]["visible"],
                json!(false),
                "{what}: second press hides it"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.hide-tool-window" if run_context => {
            h.invoke("workbench.toggle-run", json!(null));
            h.focus_run(false);
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["run"]["visible"], json!(false), "{what}");
            assert_eq!(
                s["tool_windows"]["project"]["visible"],
                json!(true),
                "{what}"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" | "run.rerun" | "run.close-tab" => {
            check_tab_key(h, &b.command, keys, &what, console_context)
        }
        "run.run" | "run.current-file" => {
            support::set_script(&support::script("echo done", "echo done"));
            h.press(keys);
            let s = h.wait_run("the run", |s| s["tabs"][0]["status"] == json!("terminated"));
            assert_eq!(s["tabs"][0]["title"], json!("a"), "{what}");
            assert_eq!(s["visible"], json!(true), "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "run.configurations" => {
            // Choose Run Configuration opens the run widget's dropdown.
            assert!(h.run()["menu"].is_null(), "{what}");
            h.press(keys);
            assert!(!h.run()["menu"].is_null(), "{what}: the dropdown opens");
        }
        "run.stop" => {
            h.start_run(&support::long_running(), "started");
            h.press(keys);
            h.wait_run("the stop", |s| s["tabs"][0]["status"] != json!("running"));
        }
        "run.console.previous-link" | "run.console.next-link" => {
            let a = h.root.join("a.py").display().to_string();
            let b_path = h.root.join("b.py").display().to_string();
            h.start_run(
                &support::script(
                    &format!("echo 'at {a}:1'; echo 'at {b_path}:1'"),
                    &format!("echo at {a}:1& echo at {b_path}:1"),
                ),
                "",
            );
            h.focus_run(false);
            h.press(keys);
            // From no link, Down goes to the first link, Up to the last.
            let expected = if b.command.ends_with("next-link") {
                "a.py"
            } else {
                "b.py"
            };
            let path = h.state()["editor"]["path"].as_str().unwrap().to_owned();
            assert!(path.ends_with(expected), "{what}: {path}");
        }
        _ => return false,
    }
    true
}
