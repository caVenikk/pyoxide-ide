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
            h.wait_idle_shell();
            h.press(keys);
            h.wait_terminal(what, |s| tabs(s) == before);
        }
        "terminal.show-tab-list" => {
            let before = tabs(&h.open_terminal(None));
            // The list needs more than one tab.
            if before == 1 {
                h.press(keys);
                assert!(h.terminal()["tab_list"].is_null(), "{what}: one tab");
            }
            h.invoke("terminal.new-tab", json!(null));
            let s = h.wait_terminal("a second tab", |s| tabs(s) == before + 1);
            let (n, selected) = (tabs(&s), s["selected"].as_u64().unwrap());
            let tab_names: Vec<String> = s["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_owned())
                .collect();
            let list_of = |s: &Value| s["tab_list"]["levels"][0].clone();
            h.press(keys);
            let level = list_of(&h.terminal());
            assert!(!level.is_null(), "{what}: opens the list of tabs");
            let names: Vec<&str> = level["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["label"].as_str().unwrap())
                .collect();
            assert_eq!(names, tab_names, "{what}: a row per tab, by name");
            assert_eq!(level["selected"], json!(selected), "{what}: the selected tab");
            // Choosing another row selects its tab, and the focus follows.
            let target = if selected + 1 < n as u64 { selected + 1 } else { selected - 1 };
            h.press(if target > selected { "down" } else { "up" });
            h.press("enter");
            let s = h.wait_terminal(what, |s| s["tab_list"].is_null());
            assert_eq!(s["selected"], json!(target), "{what}: the chosen tab");
            assert!(h.terminal_focused(), "{what}: the focus is in the tool window");
            // The list opens at the selected tab again; the key is the last
            // one pressed.
            h.press(keys);
            let level = list_of(&h.terminal());
            assert_eq!(level["selected"], json!(target), "{what}: the new selected tab");
            h.invoke("menu.cancel", json!(null));
            assert!(h.terminal()["tab_list"].is_null(), "{what}: cancelling closes it");
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

/// The keys of the rename balloon: Enter renames (an empty name is refused
/// with an error outline, the balloon stays), Escape cancels; the focus
/// goes back to the session.
fn check_rename(h: &mut H, command: &str, keys: &str, what: &str) {
    let name = |s: &Value| {
        let i = s["selected"].as_u64().unwrap() as usize;
        s["tabs"][i]["name"].as_str().unwrap().to_owned()
    };
    let before = name(&h.open_terminal(None));
    h.invoke("terminal.rename-tab", json!(null));
    let s = h.terminal();
    assert_eq!(s["rename"]["text"], json!(before), "{what}: the balloon has the name");
    if command == "terminal.rename-confirm" {
        h.invoke("editor.select-all", json!(null));
        h.invoke("editor.backspace", json!(null));
        h.press(keys);
        assert_eq!(h.terminal()["rename"]["error"], json!(true), "{what}: empty");
        h.invoke("editor.type", json!({"text": "build"}));
        assert_eq!(h.terminal()["rename"]["error"], json!(false), "{what}: typed");
        h.press(keys);
        let s = h.terminal();
        assert!(s["rename"].is_null(), "{what}: the balloon closes");
        assert_eq!(name(&s), "build", "{what}");
        h.invoke("terminal.rename-tab", json!({"name": before}));
    } else {
        h.invoke("editor.type", json!({"text": "other"}));
        h.press(keys);
        let s = h.terminal();
        assert!(s["rename"].is_null(), "{what}: the balloon closes");
        assert_eq!(name(&s), before, "{what}: the name stays");
    }
    assert!(h.terminal_focused(), "{what}: the focus is in the tool window");
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
    /// typing `input`, and waits for the shell's prompt (and, without
    /// input, for the shell to be idle).
    fn open_terminal(&mut self, input: Option<&str>) -> Value {
        self.invoke(
            "terminal.open",
            json!({"shell": TEST_SHELL, "input": input}),
        );
        let state = self.wait_prompt();
        if input.is_none() {
            self.wait_idle_shell();
            return self.terminal();
        }
        state
    }

    /// Waits for the selected session's prompt.
    fn wait_prompt(&mut self) -> Value {
        self.wait_terminal("the shell's prompt", |s| {
            s["started"] == json!(true)
                && s["lines"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|l| !l.as_str().unwrap().is_empty()))
        })
    }

    /// Waits until the selected session's shell has its prompt and no
    /// child process: its start-up scripts may run programs, and a tab
    /// closed meanwhile asks first.
    fn wait_idle_shell(&mut self) {
        self.wait_prompt();
        let session = self
            .workbench(|w, cx| w.terminal_session(cx))
            .expect("a session");
        for _ in 0..2000 {
            if !self.cx.update(|cx| {
                session
                    .read(cx)
                    .session()
                    .is_some_and(|s| s.has_running_commands())
            }) {
                return;
            }
            support::tick(self.cx);
        }
        panic!("the shell did not get idle");
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

/// "Override IDE shortcuts": a focused terminal takes the key of an IDE
/// shortcut whose action is not allowed (macOS Ctrl+V, the VCS Operations
/// popup, goes to the shell; so does Ctrl+R, Run: the shell's history
/// search), keeps the keys of allowed actions for the IDE (Alt+F12 hides the
/// tool window, Cmd+Shift+O is Go to File) and leaves every key to the IDE
/// when the option is off.
#[gpui::test]
fn a_focused_terminal_takes_the_keys_of_ide_shortcuts(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, keymap::MACOS);
    h.open_terminal(None);
    // A program that shows what it reads: the terminal echoes Ctrl+R as
    // `^R` (a Windows console does not echo it).
    if cfg!(unix) {
        h.invoke("terminal.type", json!({"text": "cat\r"}));
    }
    h.press("ctrl-r");
    let dispatched = h.dispatched.borrow().clone();
    assert!(
        !dispatched.iter().any(|a| a == "run.run"),
        "Ctrl+R went to the shell, not to Run: {dispatched:?}"
    );
    if cfg!(unix) {
        h.wait_terminal("the echo of Ctrl+R", |s| {
            s["lines"]
                .as_array()
                .is_some_and(|l| l.iter().any(|l| l.as_str().unwrap().contains("^R")))
        });
        // End of input ends `cat`.
        h.invoke("terminal.type", json!({"text": "\u{4}"}));
    }
    h.press("ctrl-v");
    let dispatched = h.dispatched.borrow().clone();
    assert!(
        !dispatched.iter().any(|a| a == "vcs.operations-popup"),
        "the key went to the shell: {dispatched:?}"
    );
    // An allowed shortcut stays the IDE's.
    h.press("cmd-shift-o");
    let dispatched = h.dispatched.borrow().clone();
    assert!(
        dispatched.iter().any(|a| a == "search-everywhere.go-to-file"),
        "Cmd+Shift+O reached the IDE: {dispatched:?}"
    );
    h.invoke("search-everywhere.close", json!(null));
    h.open_terminal(None);
    h.press("alt-f12");
    assert!(!h.terminal_visible(), "Alt+F12 hides the terminal");
    h.invoke(
        "settings.set",
        json!({"key": "terminal.override-ide-shortcuts", "value": false}),
    );
    h.open_terminal(None);
    h.press("ctrl-v");
    let dispatched = h.dispatched.borrow().clone();
    assert!(
        dispatched.iter().any(|a| a == "vcs.operations-popup"),
        "without the option the IDE takes the key: {dispatched:?}"
    );
}

/// A command that keeps the shell busy for a minute.
const BUSY: &str = if cfg!(windows) {
    "ping -n 60 127.0.0.1 >nul\r"
} else {
    "sleep 60\r"
};

impl H<'_> {
    /// Starts [`BUSY`] in the selected session and waits until the shell has
    /// it as a child process.
    fn start_busy_command(&mut self) {
        self.invoke("terminal.type", json!({"text": BUSY}));
        let session = self
            .workbench(|w, cx| w.terminal_session(cx))
            .expect("a session");
        for _ in 0..2000 {
            support::tick(self.cx);
            if self.cx.update(|cx| {
                session
                    .read(cx)
                    .session()
                    .is_some_and(|s| s.has_running_commands())
            }) {
                return;
            }
        }
        panic!("the command did not start");
    }

    /// Waits for the message dialog (at most about 20 s).
    fn wait_dialog(&mut self) -> Value {
        for _ in 0..2000 {
            support::tick(self.cx);
            self.draw();
            let dialog = self.state()["dialog"].clone();
            if !dialog.is_null() {
                return dialog;
            }
        }
        panic!("no dialog");
    }
}

/// A tab whose shell runs a command asks before it closes (the reference's
/// `TerminateRemoteProcessDialog`: "Process 'Terminal Local' Is Running",
/// Terminate or Cancel, "Don't ask again"); Cancel keeps the tab, Terminate
/// closes it, and Terminate with "Don't ask again" closes later tabs at once.
#[gpui::test]
fn a_tab_with_a_running_command_asks_before_it_closes(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, keymap::MACOS);
    h.open_terminal(None);
    h.start_busy_command();
    h.press("cmd-w");
    let dialog = h.wait_dialog();
    assert_eq!(dialog["title"], json!("Process 'Terminal Local' Is Running"));
    assert_eq!(
        dialog["message"],
        json!("Do you want to terminate the process 'Terminal Local'?")
    );
    assert_eq!(dialog["buttons"], json!(["Cancel", "Terminate"]));
    assert_eq!(dialog["default"], json!("Terminate"));
    assert_eq!(dialog["option"]["label"], json!("Don't ask again"));
    // Cancel: the tab stays.
    h.press("escape");
    assert!(h.state()["dialog"].is_null());
    assert_eq!(h.terminal()["tabs"].as_array().unwrap().len(), 1);
    assert!(h.terminal_focused(), "the focus returns to the terminal");
    // Terminate (the default button) ends the session with the tab; the
    // last tab closed hides the tool window.
    h.press("cmd-w");
    h.wait_dialog();
    h.press("enter");
    h.wait_terminal("the tab closed", |s| s["tabs"] == json!([]));
    assert!(h.state()["dialog"].is_null());
    assert!(!h.terminal_visible());
    // Terminate with "Don't ask again" is remembered.
    h.open_terminal(None);
    h.start_busy_command();
    h.press("cmd-w");
    h.wait_dialog();
    h.invoke("dialog.toggle-option", json!(null));
    h.press("enter");
    h.wait_terminal("the tab closed", |s| s["tabs"] == json!([]));
    h.open_terminal(None);
    h.start_busy_command();
    h.press("cmd-w");
    h.wait_terminal("the tab closed at once", |s| s["tabs"] == json!([]));
    assert!(h.state()["dialog"].is_null());
}

/// A tab whose shell is idle closes at once, without a dialog.
#[gpui::test]
fn a_tab_with_an_idle_shell_closes_at_once(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, keymap::MACOS);
    let before = h.open_terminal(None)["tabs"].as_array().unwrap().len();
    h.invoke("terminal.new-tab", json!(null));
    h.wait_terminal("a second tab", |s| {
        s["tabs"].as_array().unwrap().len() == before + 1
    });
    h.wait_idle_shell();
    h.press("cmd-w");
    h.wait_terminal("the tab closed", |s| {
        s["tabs"].as_array().unwrap().len() == before
    });
    assert!(h.state()["dialog"].is_null());
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
        | "terminal.previous-tab"
        | "terminal.show-tab-list" => check_tool_window(h, &b.command, keys, &what),
        "terminal.rename-confirm" | "terminal.rename-cancel" => {
            check_rename(h, &b.command, keys, &what)
        }
        _ => return false,
    }
    true
}

impl H<'_> {
    /// The selected tab's title in the header.
    fn selected_tab_name(&mut self) -> String {
        let s = self.terminal();
        let i = s["selected"].as_u64().unwrap() as usize;
        s["tabs"][i]["name"].as_str().unwrap().to_owned()
    }
}

/// While a command runs (the shell integration's marks C to D) a tab shows
/// the title the program set, OSC 1 included (the reference's emulator
/// takes the icon name as the title), cut to 30 characters in the middle;
/// afterwards its name again. A name the user gave stays.
#[gpui::test]
#[cfg_attr(windows, ignore = "cmd.exe has no printf")]
fn a_tab_shows_the_program_title_while_a_command_runs(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, keymap::MACOS);
    h.open_terminal(None);
    let name = h.selected_tab_name();
    // The command prints the mark C and the titles, then runs for a while;
    // the next prompt ends it (/bin/sh's integration marks only prompts).
    let run = |titles: &str| format!("printf '\\033]133;C\\007{titles}'; sleep 5\r");
    h.invoke(
        "terminal.type",
        json!({"text": run("\\033]2;sleep 100\\007\\033]1;sleep\\007")}),
    );
    h.wait_terminal("the program's title", |s| s["tabs"][0]["name"] == json!("sleep"));
    h.wait_terminal("the name again", |s| s["tabs"][0]["name"] == json!(name));
    let long = "python -m http.server 8000 --bind 127.0.0.1";
    h.invoke(
        "terminal.type",
        json!({"text": run(&format!("\\033]0;{long}\\007"))}),
    );
    let s = h.wait_terminal("the long title", |s| s["tabs"][0]["full_title"] == json!(long));
    assert_eq!(s["tabs"][0]["name"], json!("python -m http…-bind 127.0.0.1"));
    h.wait_terminal("the name again", |s| s["tabs"][0]["name"] == json!(name));
    // A renamed tab keeps its name.
    h.invoke("terminal.rename-tab", json!({"name": "build"}));
    h.invoke("terminal.type", json!({"text": run("\\033]1;vim\\007")}));
    let s = h.wait_terminal("the program's title set", |s| {
        s["tabs"][0]["command_running"] == json!(true) && s["tabs"][0]["title"] == json!("vim")
    });
    assert_eq!(s["tabs"][0]["name"], json!("build"));
}

/// The header's ⋮ menu has the reference's items in its order (without
/// the AI agents' toggle); Group Tabs shows once there were several tabs
/// and groups them; Close All closes every tab; Remove from Sidebar hides
/// the tool window and its button, which comes back with the tool window.
#[gpui::test]
fn the_options_menu(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, keymap::MACOS);
    h.open_terminal(None);
    let items = |h: &mut H| -> Vec<(String, bool)> {
        h.terminal()["options_menu"]["levels"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["hidden"] != json!(true))
            .map(|r| {
                let label = if r["separator"] == json!(true) {
                    "-".to_owned()
                } else {
                    r["label"].as_str().unwrap().to_owned()
                };
                (label, r["enabled"] == json!(true))
            })
            .collect()
    };
    h.invoke("terminal.options-menu", json!(null));
    let one_tab = items(&mut h);
    let labels: Vec<&str> = one_tab.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Terminal Engine",
            "Settings",
            "-",
            "Close All",
            "View Mode",
            "Move to",
            "Resize",
            "-",
            "Remove from Sidebar",
        ]
    );
    h.invoke("menu.cancel", json!(null));
    assert!(h.terminal()["options_menu"].is_null());
    let tabs = |s: &Value| s["tabs"].as_array().map_or(0, Vec::len);
    h.invoke("terminal.new-tab", json!(null));
    h.wait_terminal("a second tab", |s| tabs(s) == 2);
    h.wait_idle_shell();
    h.invoke("terminal.options-menu", json!(null));
    let labels: Vec<String> = items(&mut h).into_iter().map(|(l, _)| l).collect();
    assert_eq!(labels[4], "Group Tabs", "{labels:?}");
    h.invoke("menu.cancel", json!(null));
    h.invoke("terminal.group-tabs", json!(null));
    assert_eq!(h.terminal()["grouped"], json!(true));
    h.invoke("terminal.group-tabs", json!(null));
    assert_eq!(h.terminal()["grouped"], json!(false));
    // Close All: idle shells close at once; the tool window hides.
    h.invoke("terminal.close-all-tabs", json!(null));
    h.wait_terminal("no tab", |s| tabs(s) == 0);
    assert!(!h.terminal_visible());
    // Remove from Sidebar.
    h.open_terminal(None);
    h.invoke("terminal.remove-from-sidebar", json!(null));
    let terminal = h.state()["tool_windows"]["terminal"].clone();
    assert_eq!(terminal["visible"], json!(false), "{terminal}");
    assert_eq!(terminal["registered"], json!(false), "{terminal}");
    h.invoke("workbench.toggle-terminal", json!(null));
    let terminal = h.state()["tool_windows"]["terminal"].clone();
    assert_eq!(terminal["visible"], json!(true), "{terminal}");
    assert_eq!(terminal["registered"], json!(true), "{terminal}");
}
