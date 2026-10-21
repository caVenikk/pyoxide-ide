//! The terminal: the bindings of `keymaps/*/terminal.toml`.

use super::*;

area_tests!("terminal");

/// A plain shell for the terminal checks (not the user's shell and its
/// startup files).
const TEST_SHELL: &str = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };

/// Prints numbered lines `line0` to `line99` in [`TEST_SHELL`], then
/// `mark<n>` (the command line shows the mark quoted, so only the output
/// matches it).
fn print_lines(n: usize) -> String {
    if cfg!(windows) {
        format!("for /L %i in (0,1,99) do @echo line%i\r@echo m^ark{n}\r")
    } else {
        format!("i=0; while [ $i -lt 100 ]; do echo line$i; i=$((i+1)); done; echo m''ark{n}\r")
    }
}

static PRINTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn check_terminal(h: &mut H, command: &str, keys: &str, what: &str) {
    let offset = |s: &Value| s["display_offset"].as_u64().unwrap();
    match command {
        "terminal.copy" => {
            h.terminal_with_history();
            h.invoke("terminal.select-all", json!(null));
            h.wait_terminal("a selection", |s| !s["selection"].is_null());
            h.cx.write_to_clipboard(ClipboardItem::new_string("old".into()));
            h.press(keys);
            let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert!(
                copied.as_deref().is_some_and(|t| t.contains("line42")),
                "{what}: {copied:?}"
            );
        }
        "terminal.paste" => {
            h.open_terminal(None);
            h.cx.write_to_clipboard(ClipboardItem::new_string("pasted-text".into()));
            h.press(keys);
            h.wait_terminal(what, |s| {
                s["lines"].as_array().is_some_and(|l| {
                    l.iter()
                        .any(|l| l.as_str().unwrap().contains("pasted-text"))
                })
            });
            // Leave a clean prompt.
            h.invoke("terminal.open", json!({"input": "\r"}));
        }
        "terminal.select-all" => {
            h.terminal_with_history();
            h.press(keys);
            let s = h.wait_terminal(what, |s| !s["selection"].is_null());
            assert!(s["selection"].as_str().unwrap().contains("line0"), "{what}");
        }
        "terminal.clear" => {
            h.terminal_with_history();
            h.press(keys);
            let s = h.wait_terminal(what, |s| s["history"] == json!(0));
            assert_eq!(s["cursor"]["line"], json!(0), "{what}: the prompt moves up");
        }
        "terminal.line-up" | "terminal.page-up" => {
            let before = history(&h.terminal_with_history());
            h.press(keys);
            let s = h.wait_terminal(what, |s| offset(s) > 0);
            let expected = if command == "terminal.line-up" {
                1
            } else {
                s["rows"].as_u64().unwrap()
            };
            // Output that arrives while the view is scrolled up (the shell's
            // next prompt, late on ConPTY) moves the offset with it; output
            // that arrived before the key does not.
            let late = history(&s) - before;
            let o = offset(&s);
            assert!(o >= expected && o <= expected + late, "{what}: {o}, {expected} + {late}");
        }
        "terminal.line-down" | "terminal.page-down" => {
            h.terminal_with_history();
            h.invoke("terminal.page-up", json!(null));
            let scrolled = h.wait_terminal("scrolled up", |s| offset(s) > 0);
            let up = offset(&scrolled);
            h.press(keys);
            let s = h.wait_terminal(what, |s| offset(s) < up);
            if command == "terminal.line-down" {
                let late = history(&s) - history(&scrolled);
                let o = offset(&s);
                assert!(o + 1 >= up && o < up + late, "{what}: {o}, {up} - 1 + {late}");
            } else {
                assert_eq!(offset(&s), 0, "{what}");
            }
            h.invoke("terminal.page-down", json!(null));
        }
        _ => unreachable!("{command}"),
    }
}

/// The keys of the Terminal tool window and its tabs.
fn check_tool_window(h: &mut H, command: &str, keys: &str, what: &str) {
    let tabs = |s: &Value| s["tabs"].as_array().map_or(0, Vec::len);
    match command {
        "workbench.toggle-terminal" => {
            h.open_terminal(None);
            h.press(keys);
            assert!(!h.terminal_visible(), "{what}: hides the focused terminal");
            h.press(keys);
            assert!(h.terminal_visible(), "{what}: shows it again");
            assert!(h.terminal_focused(), "{what}: with the focus");
        }
        "workbench.hide-tool-window" => {
            h.open_terminal(None);
            h.press(keys);
            assert!(!h.terminal_visible(), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes to the editor");
        }
        "terminal.new-tab" => {
            let before = tabs(&h.open_terminal(None));
            h.press(keys);
            let s = h.wait_terminal(what, |s| tabs(s) == before + 1);
            assert_eq!(s["selected"], json!(before), "{what}: the new tab is selected");
            assert!(h.terminal_focused(), "{what}: with the focus");
        }
        "terminal.close-tab" => {
            let before = tabs(&h.open_terminal(None));
            h.invoke("terminal.new-tab", json!(null));
            h.wait_terminal("a second tab", |s| tabs(s) == before + 1);
            h.press(keys);
            h.wait_terminal(what, |s| tabs(s) == before);
        }
        "terminal.next-tab" | "terminal.previous-tab" => {
            let before = tabs(&h.open_terminal(None));
            h.invoke("terminal.new-tab", json!(null));
            let s = h.wait_terminal("a second tab", |s| tabs(s) == before + 1);
            let n = tabs(&s) as u64;
            let selected = s["selected"].as_u64().unwrap();
            h.press(keys);
            let expected = if command == "terminal.next-tab" {
                (selected + 1) % n
            } else {
                (selected + n - 1) % n
            };
            h.wait_terminal(what, |s| s["selected"] == json!(expected));
        }
        _ => unreachable!("{command}"),
    }
}

/// The number of scrollback lines in a terminal state.
fn history(s: &Value) -> u64 {
    s["history"].as_u64().unwrap()
}

impl H<'_> {
    /// The Terminal tool window's state.
    fn terminal(&mut self) -> Value {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke("terminal.state", json!(null), window, cx)
            })
            .unwrap()
            .unwrap_or(Value::Null)
    }

    fn terminal_visible(&mut self) -> bool {
        self.cx.run_until_parked();
        self.state()["tool_windows"]["terminal"]["visible"] == json!(true)
    }

    /// The focus is in the Terminal tool window.
    fn terminal_focused(&mut self) -> bool {
        let view = self.workbench(|w, _| w.terminal_window().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                view.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }

    /// Waits (at most about 20 s) for a terminal state that `f` accepts;
    /// frames are drawn meanwhile (a frame takes the session's snapshot).
    fn wait_terminal(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            support::tick(self.cx);
            self.draw();
            last = self.terminal();
            if f(&last) {
                return last;
            }
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    /// Shows the terminal (a plain shell) with the focus in it, optionally
    /// typing `input`, and waits for the shell's prompt.
    fn open_terminal(&mut self, input: Option<&str>) -> Value {
        self.invoke(
            "terminal.open",
            json!({"shell": TEST_SHELL, "input": input}),
        );
        self.wait_terminal("the shell's prompt", |s| {
            s["started"] == json!(true)
                && s["lines"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|l| !l.as_str().unwrap().is_empty()))
        })
    }

    /// The terminal with `line0` to `line99` printed (a scrollback).
    fn terminal_with_history(&mut self) -> Value {
        let n = PRINTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.open_terminal(Some(&print_lines(n)));
        let mark = format!("mark{n}");
        self.wait_terminal("the printed lines", |s| {
            s["history"].as_u64().is_some_and(|h| h > 10)
                && s["lines"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|l| l.as_str().unwrap() == mark))
        })
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "terminal.copy"
        | "terminal.paste"
        | "terminal.select-all"
        | "terminal.clear"
        | "terminal.line-up"
        | "terminal.line-down"
        | "terminal.page-up"
        | "terminal.page-down" => check_terminal(h, &b.command, keys, &what),
        "workbench.toggle-terminal"
        | "workbench.hide-tool-window"
        | "terminal.new-tab"
        | "terminal.close-tab"
        | "terminal.next-tab"
        | "terminal.previous-tab" => check_tool_window(h, &b.command, keys, &what),
        _ => return false,
    }
    true
}
