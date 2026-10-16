//! The `debug.*` engine commands (ADR 0001, 0015): the same functions the
//! UI calls, for the CLI and the control channel. Files are relative to
//! the project root or absolute; lines count from 1. Commands on a session
//! act on the current one (the last started that has not ended) unless a
//! `session` id is given. Titles follow the reference's action texts
//! (`XDebuggerBundle`, `ActionsBundle`).

use commands::{CommandError, CommandMeta, Registry, typed};
use dap_client::EvaluateContext;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::breakpoints::{ExceptionBreakpoint, SuspendPolicy, Toggled};
use crate::debugger::Debugger;
use crate::session::{DebugError, Pending, Session, SessionId};
use crate::storage;

macro_rules! meta {
    ($id:literal, $title:literal, $summary:literal, $example:literal) => {
        CommandMeta {
            id: $id,
            title: $title,
            summary: $summary,
            args_example: $example,
        }
    };
}

#[derive(Deserialize)]
struct LineArgs {
    file: String,
    line: u32,
    #[serde(default)]
    temporary: bool,
}

#[derive(Deserialize)]
struct EditArgs {
    file: String,
    line: u32,
    enabled: Option<bool>,
    suspend: Option<String>,
    /// An empty text removes the condition.
    condition: Option<String>,
    log_message: Option<bool>,
    log_stack: Option<bool>,
    log_expression: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionArgs {
    class: String,
    /// The breakpoint type; the type of the default exception breakpoint
    /// when left out.
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MuteArgs {
    muted: Option<bool>,
}

#[derive(Deserialize)]
struct WatchArgs {
    expression: Option<String>,
    index: Option<usize>,
    /// The watches' key (a configuration type); the current session's
    /// when left out.
    key: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SessionArgs {
    session: Option<u64>,
}

#[derive(Deserialize)]
struct EvaluateArgs {
    expression: String,
    session: Option<u64>,
}

#[derive(Deserialize)]
struct SetValueArgs {
    /// Names from a scope of the selected frame: `["items", "0"]`.
    path: Vec<String>,
    value: String,
    session: Option<u64>,
}

fn error(e: DebugError) -> CommandError {
    CommandError::new("debug.failed", e.to_string())
}

fn session(debugger: &Debugger, id: Option<u64>) -> Result<Session, CommandError> {
    match id {
        Some(id) => debugger.session(SessionId(id)),
        None => debugger.current_session(),
    }
    .ok_or_else(|| CommandError::not_found("No debug session"))
}

fn file_of(debugger: &Debugger, file: &str) -> Result<std::path::PathBuf, CommandError> {
    storage::resolve_path(file, debugger.root())
        .ok_or_else(|| CommandError::invalid_args("an empty file"))
}

/// Registers every `debug.*` engine command.
pub fn register(registry: &mut Registry, debugger: Debugger) {
    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.state",
            "Debugger State",
            "Returns the debugger state as JSON: breakpoints with their verification, exception breakpoints, watches, mute and the sessions with threads, frames and loaded variables.",
            "{}"
        ),
        move |_: Value| {
            let d = d.clone();
            async move { Ok::<_, CommandError>(d.state_json()) }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.toggle-breakpoint",
            "Toggle Line Breakpoint",
            "Adds a line breakpoint, or removes the one on the line; temporary ones are removed once hit.",
            r#"{"file": "main.py", "line": 7, "temporary": false}"#
        ),
        move |a: LineArgs| {
            let d = d.clone();
            async move {
                let file = file_of(&d, &a.file)?;
                let added = matches!(
                    d.toggle_breakpoint(&file, a.line.max(1), a.temporary),
                    Toggled::Added(_)
                );
                Ok::<_, CommandError>(json!({"added": added}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.toggle-breakpoint-enabled",
            "Enable/Disable Breakpoint",
            "Enables or disables the breakpoint on a line.",
            r#"{"file": "main.py", "line": 7}"#
        ),
        move |a: LineArgs| {
            let d = d.clone();
            async move {
                let file = file_of(&d, &a.file)?;
                let id = d
                    .breakpoints()
                    .at(&file, a.line)
                    .map(|b| b.id)
                    .ok_or_else(|| CommandError::not_found("No breakpoint on the line"))?;
                let mut enabled = false;
                d.update_breakpoint(id, |b| {
                    b.enabled = !b.enabled;
                    enabled = b.enabled;
                });
                Ok::<_, CommandError>(json!({"enabled": enabled}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.edit-breakpoint",
            "Edit Breakpoint",
            "Changes the breakpoint on a line: enabled, suspend policy (all, thread, none), condition and logging.",
            r#"{"file": "main.py", "line": 7, "condition": "x > 1", "suspend": "thread"}"#
        ),
        move |a: EditArgs| {
            let d = d.clone();
            async move {
                let file = file_of(&d, &a.file)?;
                let id = d
                    .breakpoints()
                    .at(&file, a.line)
                    .map(|b| b.id)
                    .ok_or_else(|| CommandError::not_found("No breakpoint on the line"))?;
                let suspend = match &a.suspend {
                    Some(text) => Some(SuspendPolicy::parse(text).ok_or_else(|| {
                        CommandError::invalid_args("suspend is all, thread or none")
                    })?),
                    None => None,
                };
                let text = |t: String| Some(t).filter(|t| !t.trim().is_empty());
                d.update_breakpoint(id, |b| {
                    if let Some(enabled) = a.enabled {
                        b.enabled = enabled;
                    }
                    if let Some(suspend) = suspend {
                        b.suspend = suspend;
                    }
                    if let Some(condition) = a.condition {
                        b.condition = text(condition);
                    }
                    if let Some(log) = a.log_message {
                        b.log_message = log;
                    }
                    if let Some(log) = a.log_stack {
                        b.log_stack = log;
                    }
                    if let Some(expression) = a.log_expression {
                        b.log_expression = text(expression);
                    }
                });
                Ok::<_, CommandError>(json!({"edited": true}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.add-exception-breakpoint",
            "Add Exception Breakpoint",
            "Adds an exception breakpoint for an exception class.",
            r#"{"class": "ValueError"}"#
        ),
        move |a: ExceptionArgs| {
            let d = d.clone();
            async move {
                let kind = a
                    .kind
                    .or_else(|| {
                        d.breakpoints()
                            .exception()
                            .iter()
                            .find(|b| b.default)
                            .map(|b| b.kind.clone())
                    })
                    .ok_or_else(|| CommandError::invalid_args("the breakpoint type is missing"))?;
                d.add_exception_breakpoint(ExceptionBreakpoint::new(kind, a.class));
                Ok::<_, CommandError>(json!({"added": true}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.mute-breakpoints",
            "Mute Breakpoints",
            "Mutes or unmutes every breakpoint (toggles without an argument).",
            r#"{"muted": true}"#
        ),
        move |a: MuteArgs| {
            let d = d.clone();
            async move {
                let muted = a.muted.unwrap_or(!d.is_muted());
                d.set_muted(muted);
                Ok::<_, CommandError>(json!({"muted": muted}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.new-watch",
            "New Watch",
            "Adds a watch expression for the current session's configuration type.",
            r#"{"expression": "len(items)"}"#
        ),
        move |a: WatchArgs| {
            let d = d.clone();
            async move {
                let key = watch_key(&d, a.key)?;
                let expression = a
                    .expression
                    .ok_or_else(|| CommandError::invalid_args("an expression is needed"))?;
                let added = d.add_watch(&key, &expression, a.index);
                Ok::<_, CommandError>(json!({"added": added, "watches": d.watches(&key)}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.remove-watch",
            "Remove Watch",
            "Removes a watch by its index or its expression.",
            r#"{"expression": "len(items)"}"#
        ),
        move |a: WatchArgs| {
            let d = d.clone();
            async move {
                let key = watch_key(&d, a.key)?;
                let index = match (a.index, a.expression) {
                    (Some(index), _) => Some(index),
                    (None, Some(expression)) => {
                        d.watches(&key).iter().position(|w| *w == expression)
                    }
                    (None, None) => None,
                };
                let removed = index.is_some_and(|i| d.remove_watch(&key, i));
                Ok::<_, CommandError>(json!({"removed": removed, "watches": d.watches(&key)}))
            }
        },
    ));

    macro_rules! action {
        ($id:literal, $title:literal, $summary:literal, $method:ident) => {
            let d = debugger.clone();
            registry.register(typed(
                meta!($id, $title, $summary, "{}"),
                move |a: SessionArgs| {
                    let d = d.clone();
                    async move {
                        let s = session(&d, a.session)?;
                        let pending: Pending<()> = s.$method();
                        pending.await.map_err(error)?;
                        Ok::<_, CommandError>(json!({"state": s.phase().as_str()}))
                    }
                },
            ));
        };
    }
    action!(
        "debug.resume",
        "Resume Program",
        "Resumes the suspended program.",
        resume
    );
    action!(
        "debug.pause",
        "Pause Program",
        "Suspends the running program.",
        pause
    );
    action!(
        "debug.step-over",
        "Step Over",
        "Runs to the next line of the current function.",
        step_over
    );
    action!(
        "debug.step-into",
        "Step Into",
        "Steps into the function called on the line.",
        step_into
    );
    action!(
        "debug.step-into-my-code",
        "Step Into My Code",
        "Steps into the next function of the project, skipping library code.",
        step_into_my_code
    );
    action!(
        "debug.step-out",
        "Step Out",
        "Runs to the caller of the current function.",
        step_out
    );

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.stop",
            "Stop",
            "Stops the debug session: the program and the debugger end.",
            "{}"
        ),
        move |a: SessionArgs| {
            let d = d.clone();
            async move {
                let s = session(&d, a.session)?;
                let done = s.stop();
                let _ = done.recv_async().await;
                Ok::<_, CommandError>(json!({"state": s.phase().as_str()}))
            }
        },
    ));

    let d = debugger.clone();
    registry.register(typed(
        meta!(
            "debug.evaluate",
            "Evaluate Expression",
            "Evaluates an expression in the selected frame of the suspended program.",
            r#"{"expression": "len(items)"}"#
        ),
        move |a: EvaluateArgs| {
            let d = d.clone();
            async move {
                let s = session(&d, a.session)?;
                let value = s
                    .evaluate(&a.expression, EvaluateContext::Repl)
                    .await
                    .map_err(error)?;
                Ok::<_, CommandError>(json!({
                    "value": value.value,
                    "type": value.type_name,
                    "has_children": value.reference > 0,
                }))
            }
        },
    ));

    let d = debugger;
    registry.register(typed(
        meta!(
            "debug.set-value",
            "Set Value",
            "Sets a variable of the selected frame, found by its path of names, to the value of an expression.",
            r#"{"path": ["items", "0"], "value": "5"}"#
        ),
        move |a: SetValueArgs| {
            let d = d.clone();
            async move {
                let s = session(&d, a.session)?;
                let (parent, variable) = s.find_variable(a.path).await.map_err(error)?;
                let value = s
                    .set_value(parent, &variable, &a.value)
                    .await
                    .map_err(error)?;
                Ok::<_, CommandError>(json!({"value": value.value, "type": value.type_name}))
            }
        },
    ));
}

fn watch_key(debugger: &Debugger, key: Option<String>) -> Result<String, CommandError> {
    key.or_else(|| {
        debugger
            .current_session()
            .map(|s| s.watches_key().to_owned())
    })
    .ok_or_else(|| CommandError::invalid_args("no debug session: name the watches' key"))
}
