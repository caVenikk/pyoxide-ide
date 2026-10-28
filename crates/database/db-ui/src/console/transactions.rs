//! The commands of a console's transactions, schema and safety checks, and
//! the answer to a session's `DecisionNeeded`: the same functions the
//! toolbar's buttons and menus, the control channel and the tests call.

use db_core::TxMode;
use db_session::{DecisionReason, Event, EventKind, Isolation, SessionId, TxPolicy};
use gpui::{App, Window};

use super::model::parse_mode;
use super::{consoles, decision, target};

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Commit the pending changes.
    Commit => "db.console.commit", "Commit",
        "Commits the transaction of the active console (manual mode).";
    /// Roll the pending changes back.
    Rollback => "db.console.rollback", "Rollback",
        "Rolls back the transaction of the active console (manual mode).";
    /// Switch the transaction mode.
    SetTxMode { mode: String } => "db.console.set-tx-mode", "Set Transaction Mode",
        "Switches the active console to `auto` or `manual` transaction mode. Leaving manual mode with pending changes asks whether to commit them, roll them back or cancel.",
        r#"{"mode": "manual"}"#;
    /// Set the isolation level.
    SetIsolation { level: String } => "db.console.set-isolation", "Set Transaction Isolation",
        "Sets the isolation level of the active console's transactions: `database-default`, `read-committed`, `repeatable-read` or `serializable` (not every level exists in every engine).",
        r#"{"level": "serializable"}"#;
    /// Switch the current schema.
    ChooseSchema { schema: String } => "db.console.set-schema", "Set Current Schema",
        "Makes `schema` the current schema of the active console's session: PostgreSQL sets the search path, Oracle the current schema. SQLite has no schemas to switch.",
        r#"{"schema": "public"}"#;
    /// Run the statements the unsafe-query banner holds.
    RunUnsafe { suppress: Option<bool> } => "db.console.run-unsafe", "Execute Unsafe Query",
        "Runs the statement the banner of the active console warns about; with `suppress` true the warning is turned off in the settings as well.",
        r#"{"suppress": false}"#;
    /// Close the unsafe-query banner.
    DismissUnsafe => "db.console.dismiss-unsafe", "Dismiss Unsafe Query Warning",
        "Closes the unsafe-query banner of the active console without running the statement.";
}

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    decision::init(cx);
    super::params_dialog::init(cx);
    cx.on_action(|_: &Commit, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.commit(cx));
        }
    });
    cx.on_action(|_: &Rollback, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.rollback(cx));
        }
    });
    cx.on_action(|a: &SetTxMode, cx| {
        let Some(mode) = parse_mode(&a.mode) else {
            eprintln!("db.console.set-tx-mode: unknown mode `{}`", a.mode);
            return;
        };
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.set_tx_mode(mode, TxPolicy::Ask, cx));
        }
    });
    cx.on_action(|a: &SetIsolation, cx| {
        let Some(level) = Isolation::from_id(&a.level) else {
            eprintln!("db.console.set-isolation: unknown level `{}`", a.level);
            return;
        };
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.set_isolation(level, cx));
        }
    });
    cx.on_action(|a: &ChooseSchema, cx| {
        if let Some(console) = target(cx) {
            let schema = a.schema.clone();
            console.update(cx, |c, cx| c.set_schema(schema, cx));
        }
    });
    cx.on_action(|a: &RunUnsafe, cx| {
        let Some(console) = target(cx) else {
            return;
        };
        let suppress = a.suppress.unwrap_or(false);
        let window = console.read(cx).window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                console.update(cx, |c, cx| c.confirm_unsafe(suppress, window, cx));
            });
        });
    });
    cx.on_action(|_: &DismissUnsafe, cx| {
        if let Some(console) = target(cx) {
            console.update(cx, |c, cx| c.dismiss_unsafe(cx));
        }
    });
}

/// The session layer waits for the person: an action would end a
/// transaction that holds changes. The question is asked over the window of
/// the first console concerned; the answer repeats the action.
pub(crate) fn route_decision(event: Event, cx: &mut App) {
    let EventKind::DecisionNeeded { reason, sessions } = event.kind else {
        return;
    };
    let concerned: Vec<_> = consoles(cx)
        .into_iter()
        .filter(|c| sessions.contains(&c.read(cx).session.id()))
        .collect();
    let Some(first) = concerned.first().cloned() else {
        return;
    };
    let pending: u32 = concerned.iter().map(|c| c.read(cx).pending()).sum();
    let message = question(reason, pending, &concerned.iter().map(|c| c.read(cx).label()).collect::<Vec<_>>());
    let window = first.read(cx).window;
    let ids: Vec<SessionId> = concerned.iter().map(|c| c.read(cx).session.id()).collect();
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window: &mut Window, cx| {
            decision::ask(message, window, cx, move |policy, cx| {
                let Some(policy) = policy else {
                    return;
                };
                repeat(reason, &ids, policy, cx);
            });
        });
    });
}

fn question(reason: DecisionReason, pending: u32, labels: &[String]) -> String {
    let changes = if pending == 1 {
        "1 uncommitted change".to_owned()
    } else {
        format!("{pending} uncommitted changes")
    };
    let what = match reason {
        DecisionReason::SwitchToAuto => "Switching to auto-commit",
        DecisionReason::Close => "Closing the session",
        DecisionReason::Disconnect | DecisionReason::IdleDisconnect => "Disconnecting",
    };
    format!(
        "{what} ends the transaction with {changes} ({}). Commit them, roll them back, or cancel?",
        labels.join(", ")
    )
}

/// Does the action again, this time with the answer.
fn repeat(reason: DecisionReason, sessions: &[SessionId], policy: TxPolicy, cx: &mut App) {
    for console in consoles(cx) {
        if !sessions.contains(&console.read(cx).session.id()) {
            continue;
        }
        console.update(cx, |c, cx| match reason {
            DecisionReason::SwitchToAuto => c.set_tx_mode(TxMode::Auto, policy, cx),
            DecisionReason::Close => c.session.close(policy),
            DecisionReason::Disconnect | DecisionReason::IdleDisconnect => {
                c.disconnect(policy);
            }
        });
        // One call ends every session of the data source.
        if matches!(
            reason,
            DecisionReason::Disconnect | DecisionReason::IdleDisconnect
        ) {
            break;
        }
    }
}

