//! SQL consoles (ADR 0021, scenarios console-run, console-results): a
//! console is a `.sql` file under the configuration folder, one folder per
//! data source, opened in the normal editor; Execute runs the statement at
//! the caret (or the selection) in the console's own session, the output
//! log and the result tabs show in the Services tool window under the
//! editor.

mod files;
mod model;
pub(crate) mod services;

use std::path::PathBuf;

use gpui::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use serde_json::{Value as Json, json};
use workbench::Workbench;

pub(crate) use model::{Console, ConsoleEvent, ConsoleKind, ConsoleTarget, Pane};

use crate::explorer;

/// The key context of the output log (its bindings in `console.toml`).
pub const OUTPUT_CONTEXT: &str = "DatabaseOutput";

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Open a console of a data source.
    NewConsole { source: Option<String> } => "db.console.new", "New Query Console",
        "Opens the query console of data source `source` (the one selected in the Database tool window when omitted) in an editor tab, creating its file in the configuration folder the first time. The data source must be shown in the Database tool window.",
        r#"{"source": "local-pg"}"#;
    /// Run the statement at the caret.
    Execute => "db.console.execute", "Execute",
        "Runs the statement at the caret of the active SQL editor, or the statements of its selection, in the session of that file: a console's own, or the session of a `.sql` file attached to a data source (its own node in the Services tool window); a file without a data source opens the data source popup. Nothing in any other editor. The output log and the result tab show in the Services tool window.";
    /// Cancel the running statement.
    Stop => "db.console.stop", "Cancel Running Statements",
        "Cancels the statement the active console is running.";
    /// Show a pane of the Services tool window.
    SelectPane { index: Option<usize> } => "db.console.select-pane", "Select Output or Result",
        "Shows the output log (no `index`) or the result tab `index` (zero-based) of the active console in the Services tool window.",
        r#"{"index": 0}"#;
    /// Close a result tab.
    CloseResult { index: usize } => "db.console.close-result", "Close Result Tab",
        "Closes the result tab `index` (zero-based) of the active console.",
        r#"{"index": 0}"#;
    /// Clear the output log.
    ClearOutput => "db.console.clear-output", "Clear Output",
        "Removes all lines of the output log of the active console.";
    /// Scroll the output log to its end.
    ScrollOutputToEnd => "db.console.scroll-output-to-end", "Scroll Output to End",
        "Scrolls the output log of the active console to its last line.";
    /// Soft-wrap the output log.
    ToggleOutputSoftWrap => "db.console.toggle-output-soft-wrap", "Soft-Wrap Output",
        "Switches soft wrapping of the output log of the active console on or off.";
    /// Expand every row of the Services tree.
    ExpandAll => "db.services.expand-all", "Expand All",
        "Expands every row of the Services tool window's tree.";
    /// Collapse every row of the Services tree.
    CollapseAll => "db.services.collapse-all", "Collapse All",
        "Collapses every row of the Services tool window's tree.";
    /// The state of the consoles.
    ConsoleState => "db.console.state", "Console State",
        "Returns the consoles of the window as JSON: file, data source, schema, whether a statement runs, the output log's lines and the result tabs with their grids.";
}

/// The consoles of all windows.
#[derive(Default)]
pub(crate) struct Consoles(pub Vec<Entity<Console>>);

impl Global for Consoles {}

pub(crate) fn consoles(cx: &App) -> Vec<Entity<Console>> {
    cx.try_global::<Consoles>()
        .map(|c| c.0.clone())
        .unwrap_or_default()
}

/// The console whose file is the active tab of `window`.
fn active_console(window: AnyWindowHandle, cx: &App) -> Option<Entity<Console>> {
    consoles(cx).into_iter().find(|c| {
        let c = c.read(cx);
        c.window == window
            && c.workbench
                .upgrade()
                .is_some_and(|wb| wb.read(cx).active_file(cx).as_deref() == Some(&c.path))
    })
}

/// The console a command goes to: the one in the active editor of the
/// active window, else the one the Services tool window shows last.
fn target(cx: &App) -> Option<Entity<Console>> {
    let window = cx.active_window();
    window
        .and_then(|w| active_console(w, cx))
        .or_else(|| consoles(cx).into_iter().next_back())
}

/// The window a command of the Services tool window goes to.
fn target_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.active_window()
        .or_else(|| target(cx).map(|c| c.read(cx).window))
}

/// Hands a session event to the console it belongs to (the module's
/// data source handles send every event to the Database tool window).
pub(crate) fn route(event: db_session::Event, cx: &mut App) {
    let (db_session::EventKind::Session { session, .. }
    | db_session::EventKind::Job { session, .. }) = &event.kind
    else {
        return;
    };
    let session = *session;
    let Some(console) = consoles(cx)
        .into_iter()
        .find(|c| c.read(cx).session.id() == session)
    else {
        return;
    };
    let window = console.read(cx).window;
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            console.update(cx, |c, cx| c.on_event(event, window, cx));
        });
    });
}

/// Opens the console of a data source in an editor tab of `workbench`.
pub(crate) fn open(
    workbench: Entity<Workbench>,
    target: ConsoleTarget,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    let path = files::console_path(crate::store(cx).console_folder(), &target.source);
    let make = cx.background_spawn({
        let path = path.clone();
        async move { files::ensure(&path) }
    });
    cx.spawn(async move |cx| {
        let made = make.await;
        let _ = cx.update(|cx| {
            let _ = window.update(cx, |_, window, cx| match made {
                Ok(()) => finish_open(workbench, target, path, window, cx),
                Err(e) => {
                    let note = ui::notification::Notification::new(
                        ui::notification::NotificationKind::Error,
                        target.source.clone(),
                        format!("Cannot create the console file: {e}"),
                    );
                    workbench.update(cx, |wb, cx| {
                        wb.notify(note, cx);
                    });
                }
            });
        });
    })
    .detach();
}

fn finish_open(
    workbench: Entity<Workbench>,
    target: ConsoleTarget,
    path: PathBuf,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    let existing = consoles(cx).into_iter().find(|c| c.read(cx).path == path);
    let console = match existing {
        Some(c) => c,
        None => {
            let weak = workbench.downgrade();
            let console = cx.new(|cx| {
                Console::new(path.clone(), target, ConsoleKind::Console, weak, window, cx)
            });
            subscribe(&console, &workbench, window, cx);
            cx.default_global::<Consoles>().0.push(console.clone());
            console
        }
    };
    workbench.update(cx, |wb, cx| {
        wb.open_file(path, window, cx);
        wb.show_tool_window(services::SERVICES, cx);
    });
    services::select(&console, window.window_handle(), cx);
}

/// The console of a file, when it has one.
pub(crate) fn console_of(path: &std::path::Path, cx: &App) -> Option<Entity<Console>> {
    consoles(cx).into_iter().find(|c| c.read(cx).path == path)
}

/// Gives the `.sql` file `path` its own session over `target` (a console
/// that is not bound to the data source by where its file lives), replacing
/// the one of another data source the file had.
pub(crate) fn attach_file(
    workbench: Entity<Workbench>,
    target: ConsoleTarget,
    path: PathBuf,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Entity<Console> {
    let kept: Vec<_> = consoles(cx)
        .into_iter()
        .filter(|c| c.read(cx).path != path)
        .collect();
    cx.default_global::<Consoles>().0 = kept;
    let weak = workbench.downgrade();
    let console = cx.new(|cx| Console::new(path, target, ConsoleKind::File, weak, window, cx));
    subscribe(&console, &workbench, window, cx);
    cx.default_global::<Consoles>().0.push(console.clone());
    console
}

/// A statement shows the Services tool window under the editor and the
/// console's node is the one it selects.
fn subscribe(
    console: &Entity<Console>,
    workbench: &Entity<Workbench>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    let weak: WeakEntity<Workbench> = workbench.downgrade();
    let sub = window.subscribe(
        console,
        cx,
        move |console, event: &ConsoleEvent, window, cx| {
            let ConsoleEvent::Started = event;
            if let Some(wb) = weak.upgrade() {
                wb.update(cx, |wb, cx| wb.show_tool_window(services::SERVICES, cx));
            }
            services::select(&console, window.window_handle(), cx);
        },
    );
    std::mem::forget(sub);
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|a: &NewConsole, cx| {
        let source = a.source.clone();
        explorer::with_target(cx, move |e, window, cx| e.open_console(source, window, cx));
    });
    cx.on_action(|_: &Execute, cx| {
        explorer::with_target(cx, |e, window, cx| e.execute(window, cx));
    });
    cx.on_action(|_: &Stop, cx| {
        let Some(console) = target(cx) else {
            return;
        };
        console.update(cx, |c, cx| c.stop(cx));
    });
    cx.on_action(|a: &SelectPane, cx| {
        let Some(console) = target(cx) else {
            return;
        };
        let pane = a.index.map_or(Pane::Output, Pane::Result);
        console.update(cx, |c, cx| c.select_pane(pane, cx));
    });
    cx.on_action(|a: &CloseResult, cx| {
        let Some(console) = target(cx) else {
            return;
        };
        let index = a.index;
        console.update(cx, |c, cx| c.close_tab(index, cx));
    });
    cx.on_action(|_: &ClearOutput, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.clear_output(cx));
        }
    });
    cx.on_action(|_: &ScrollOutputToEnd, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.scroll_output_to_end(cx));
        }
    });
    cx.on_action(|_: &ToggleOutputSoftWrap, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.toggle_soft_wrap(cx));
        }
    });
    cx.on_action(|_: &ExpandAll, cx| {
        if let Some(window) = target_window(cx) {
            services::set_all_expanded(window, true, cx);
        }
    });
    cx.on_action(|_: &CollapseAll, cx| {
        if let Some(window) = target_window(cx) {
            services::set_all_expanded(window, false, cx);
        }
    });
    ui::command::register_query(
        cx,
        "db.console.state",
        std::rc::Rc::new(|_, window, cx| Ok(state_json(window.window_handle(), cx))),
    );
    services::init(cx);
}

fn state_json(window: AnyWindowHandle, cx: &App) -> Json {
    let main = ui::dialog_window::parent_of(window, cx).unwrap_or(window);
    json!({
        "consoles": consoles(cx)
            .iter()
            .filter(|c| c.read(cx).window == main)
            .map(|c| c.read(cx).state_json(cx))
            .collect::<Vec<_>>(),
        "services": {"collapsed": services::collapsed(main, cx)},
    })
}
