//! A console's transaction controls and its current schema: the mode and
//! isolation menu, Commit and Rollback, the pending count, the schema list.
//! Every change goes to the session; what the console shows follows the
//! session's events.

use console::ContentKind;
use db_core::TxMode;
use db_session::{Command, Isolation, TxPolicy, TxStatus};
use gpui::{Context, SharedString};
use serde_json::{Value as Json, json};
use ui::icons::IconName;
use ui::menu::MenuItem;

use super::Console;
use crate::console::transactions::{ChooseSchema, SetIsolation, SetTxMode};

/// The id of a mode in commands.
pub(crate) fn mode_id(mode: TxMode) -> &'static str {
    match mode {
        TxMode::Auto => "auto",
        TxMode::Manual => "manual",
    }
}

pub(crate) fn parse_mode(id: &str) -> Option<TxMode> {
    match id {
        "auto" => Some(TxMode::Auto),
        "manual" => Some(TxMode::Manual),
        _ => None,
    }
}

impl Console {
    /// The text of the transaction button: `Tx: Auto`, `Tx: Manual`.
    pub fn tx_label(&self) -> &'static str {
        match self.tx.mode {
            TxMode::Auto => "Tx: Auto",
            TxMode::Manual => "Tx: Manual",
        }
    }

    pub fn is_manual(&self) -> bool {
        self.tx.mode == TxMode::Manual
    }

    /// Changes not committed yet.
    pub fn pending(&self) -> u32 {
        self.tx.pending
    }

    pub fn commit(&mut self, cx: &mut Context<Self>) {
        if self.is_manual() {
            self.session.commit();
            cx.notify();
        }
    }

    pub fn rollback(&mut self, cx: &mut Context<Self>) {
        if self.is_manual() {
            self.session.rollback();
            cx.notify();
        }
    }

    /// Switches the mode; leaving manual mode with changes pending asks
    /// first (`DecisionNeeded`) unless `policy` says what to do.
    pub fn set_tx_mode(&mut self, mode: TxMode, policy: TxPolicy, cx: &mut Context<Self>) {
        if mode != self.tx.mode {
            self.session.set_tx_mode(mode, policy);
            cx.notify();
        }
    }

    pub fn set_isolation(&mut self, level: Isolation, cx: &mut Context<Self>) {
        if level == self.isolation {
            return;
        }
        if !level.supported(self.backend.dialect()) {
            let text = format!(
                "{} cannot run transactions at the level {}",
                self.dialect().display_name(),
                level.title()
            );
            self.print(ContentKind::Error, &text, cx);
            return;
        }
        self.pending_isolation = Some(level);
        self.session.set_isolation(level);
    }

    /// Closes the connections of the console's data source (ends every
    /// session's transaction with `policy`).
    pub fn disconnect(&mut self, policy: TxPolicy) {
        self.handle.disconnect(policy);
    }

    /// Switches the current schema of the session (the search path, or the
    /// current schema of Oracle); the toolbar follows the session's report.
    pub fn set_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        if self.schema.as_deref() == Some(schema.as_str()) {
            return;
        }
        self.session.set_schema(schema);
        cx.notify();
    }

    /// The mode changed or a transaction opened or ended.
    pub(super) fn tx_changed(&mut self, old: TxStatus, new: TxStatus, cx: &mut Context<Self>) {
        if old.mode != new.mode {
            let text = match new.mode {
                TxMode::Manual => "manual transaction mode ON",
                TxMode::Auto => "manual transaction mode OFF",
            };
            self.print(ContentKind::System, text, cx);
        }
    }

    /// A command other than a job succeeded.
    pub(super) fn command_done(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Commit => self.print(ContentKind::System, "committed", cx),
            Command::Rollback => self.print(ContentKind::System, "rolled back", cx),
            Command::SetIsolation => {
                if let Some(level) = self.pending_isolation.take() {
                    self.isolation = level;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// A command other than a job failed: the request is forgotten.
    pub(super) fn command_failed(&mut self, command: Command) {
        if command == Command::SetIsolation {
            self.pending_isolation = None;
        }
    }

    /// The rows of the transaction popup: the mode, the isolation level and
    /// what the mode means.
    pub fn tx_menu_items(&self) -> Vec<MenuItem> {
        let dialect = self.backend.dialect();
        let mut items = vec![MenuItem::header("Transaction Mode")];
        for (mode, label) in [(TxMode::Auto, "Auto"), (TxMode::Manual, "Manual")] {
            let item = MenuItem::action(
                label,
                SetTxMode {
                    mode: mode_id(mode).to_owned(),
                },
            )
            .description(format!("Switches the console to {label} transaction mode"));
            items.push(if self.tx.mode == mode {
                item.icon(IconName::CHECKED)
            } else {
                item
            });
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::header("Transaction Isolation"));
        for level in Isolation::ALL {
            let item = MenuItem::action(
                level.title(),
                SetIsolation {
                    level: level.id().to_owned(),
                },
            )
            .description(format!("Runs the transactions at {}", level.title()))
            .enabled(level.supported(dialect));
            items.push(if self.isolation == level {
                item.icon(IconName::CHECKED)
            } else {
                item
            });
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::unavailable(match self.tx.mode {
            TxMode::Auto => "Changes submitted to the database are auto-committed",
            TxMode::Manual => "Changes submitted to the database are committed manually",
        }));
        items
    }

    /// The rows of the schema popup: the data source's database with its
    /// schemas under it (the current one checked). Empty where the engine
    /// has no schemas to switch.
    pub fn schema_menu_items(&self) -> Vec<MenuItem> {
        if self.backend.dialect() == db_core::Dialect::Sqlite {
            return Vec::new();
        }
        let model = self.handle.model().get();
        let mut schemas: Vec<(&str, bool)> = model
            .schemas()
            .map(|s| (s.name(), s.info().is_system))
            .collect();
        schemas.sort_by(|a, b| (a.1, a.0.to_lowercase()).cmp(&(b.1, b.0.to_lowercase())));
        let rows: Vec<MenuItem> = schemas
            .into_iter()
            .map(|(name, _)| {
                let item = MenuItem::action(
                    SharedString::from(name.to_owned()),
                    ChooseSchema {
                        schema: name.to_owned(),
                    },
                )
                .icon(if self.schema.as_deref() == Some(name) {
                    IconName::CHECKED
                } else {
                    crate::icons::SCHEMA
                })
                .description(format!("Makes `{name}` the current schema"));
                item
            })
            .collect();
        let database = self
            .database
            .clone()
            .unwrap_or_else(|| self.source.clone());
        vec![MenuItem::submenu(database, rows).icon(crate::icons::DATABASE_NODE)]
    }

    pub(super) fn tx_json(&self) -> Json {
        json!({
            "mode": mode_id(self.tx.mode),
            "open": self.tx.open,
            "pending": self.tx.pending,
            "isolation": self.isolation.id(),
        })
    }
}
