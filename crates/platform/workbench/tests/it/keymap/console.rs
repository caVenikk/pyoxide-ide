//! The Run and Debug consoles: the bindings of `keymaps/*/console.toml`.

use super::*;

use super::debug::{SESSION_COMMANDS, check_debug_session, check_escape};
use super::run::check_tab_key;

area_tests!("console");

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    let console_context = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("RunConsole"));
    // The Debug tool window and its console.
    let debug_context = b.context.as_deref().is_some_and(|c| c.starts_with("Debug"));
    let debug_console = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("DebugConsole"));
    match b.command.as_str() {
        "workbench.focus-editor" if debug_context => check_escape(h, keys, &what, debug_console),
        c if SESSION_COMMANDS.contains(&c) => check_debug_session(h, c, keys, &what, debug_console),
        "workbench.focus-editor" | "run.rerun" | "run.close-tab" => {
            check_tab_key(h, &b.command, keys, &what, console_context)
        }
        "run.console.send-eof" => {
            h.start_run(
                &support::script(
                    "echo started; cat; echo done",
                    "echo started& more& echo done",
                ),
                "started",
            );
            h.focus_run(true);
            h.press(keys);
            h.wait_run("the end of the input", |s| {
                s["tabs"][0]["status"] == json!("terminated")
            });
        }
        _ => return false,
    }
    true
}
