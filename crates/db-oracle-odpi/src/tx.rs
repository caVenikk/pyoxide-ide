use db_core::{Error, Result, TxMode};
use oracle::StatementType;

use crate::sqltext::{allowed_when_read_only, commits_in_auto_mode};

/// Transaction mode and read-only handling of one session, without any
/// database access: the session asks what to do and does it.
#[derive(Debug, Clone)]
pub(crate) struct TxState {
    mode: TxMode,
    read_only: bool,
    /// Manual mode on a read-only connection: the next statement has to be
    /// preceded by `SET TRANSACTION READ ONLY`, which Oracle only allows as
    /// the first statement of a transaction.
    read_only_pending: bool,
}

impl TxState {
    pub(crate) fn new(read_only: bool) -> Self {
        TxState {
            mode: TxMode::Auto,
            read_only,
            read_only_pending: false,
        }
    }

    pub(crate) fn mode(&self) -> TxMode {
        self.mode
    }

    /// Whether `statement` may run, and if so whether
    /// `SET TRANSACTION READ ONLY` has to run before it.
    pub(crate) fn admit(&self, statement: StatementType) -> Result<bool> {
        if self.read_only && !allowed_when_read_only(statement) {
            return Err(Error::query(format!(
                "this connection is read-only; {statement} statements are not allowed"
            )));
        }
        // A commit or rollback ends the transaction anyway.
        let ends_transaction = matches!(statement, StatementType::Commit | StatementType::Rollback);
        Ok(self.read_only_pending && !ends_transaction)
    }

    /// `SET TRANSACTION READ ONLY` ran.
    pub(crate) fn read_only_started(&mut self) {
        self.read_only_pending = false;
    }

    /// A statement succeeded. Returns whether the session has to commit now
    /// (automatic mode, and the statement changes data).
    pub(crate) fn statement_done(&mut self, statement: StatementType) -> bool {
        if matches!(statement, StatementType::Commit | StatementType::Rollback) {
            self.transaction_ended();
        }
        self.mode == TxMode::Auto && commits_in_auto_mode(statement)
    }

    /// A commit or rollback went through: a new transaction starts with the
    /// next statement.
    pub(crate) fn transaction_ended(&mut self) {
        self.read_only_pending = self.read_only && self.mode == TxMode::Manual;
    }

    /// Switching to automatic mode ends the open transaction, as the
    /// autocommit setting of other drivers does: the caller commits first.
    pub(crate) fn switch_needs_commit(&self, to: TxMode) -> bool {
        self.mode == TxMode::Manual && to == TxMode::Auto
    }

    pub(crate) fn set_mode(&mut self, mode: TxMode) {
        self.mode = mode;
        self.transaction_ended();
    }
}

#[cfg(test)]
mod tests {
    use db_core::ErrorKind;

    use super::*;

    #[test]
    fn auto_mode_commits_data_changes() {
        let mut tx = TxState::new(false);
        assert_eq!(tx.mode(), TxMode::Auto);
        assert!(!tx.admit(StatementType::Insert).unwrap());
        assert!(tx.statement_done(StatementType::Insert));
        assert!(tx.statement_done(StatementType::Begin));
        assert!(!tx.statement_done(StatementType::Select));
        assert!(!tx.statement_done(StatementType::Create));
        assert!(!tx.statement_done(StatementType::Commit));
    }

    #[test]
    fn manual_mode_never_commits_by_itself() {
        let mut tx = TxState::new(false);
        tx.set_mode(TxMode::Manual);
        assert!(!tx.statement_done(StatementType::Insert));
        assert!(!tx.statement_done(StatementType::Merge));
        assert!(!tx.admit(StatementType::Delete).unwrap());
    }

    #[test]
    fn leaving_manual_mode_commits() {
        let mut tx = TxState::new(false);
        assert!(!tx.switch_needs_commit(TxMode::Manual));
        assert!(!tx.switch_needs_commit(TxMode::Auto));
        tx.set_mode(TxMode::Manual);
        assert!(tx.switch_needs_commit(TxMode::Auto));
        assert!(!tx.switch_needs_commit(TxMode::Manual));
        tx.set_mode(TxMode::Auto);
        assert!(!tx.switch_needs_commit(TxMode::Auto));
    }

    #[test]
    fn read_only_auto_mode_allows_queries_only() {
        let tx = TxState::new(true);
        assert!(!tx.admit(StatementType::Select).unwrap());
        assert!(!tx.admit(StatementType::Commit).unwrap());
        for t in [
            StatementType::Insert,
            StatementType::Update,
            StatementType::Drop,
            StatementType::Begin,
            StatementType::Call,
        ] {
            let e = tx.admit(t).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Query);
            assert!(e.message.contains("read-only"), "{}", e.message);
            assert!(e.message.contains(&t.to_string()), "{}", e.message);
        }
    }

    #[test]
    fn read_only_manual_mode_starts_a_read_only_transaction_each_time() {
        let mut tx = TxState::new(true);
        // Auto mode: nothing to start.
        assert!(!tx.admit(StatementType::Select).unwrap());

        tx.set_mode(TxMode::Manual);
        assert!(tx.admit(StatementType::Select).unwrap(), "first statement");
        tx.read_only_started();
        assert!(
            !tx.admit(StatementType::Select).unwrap(),
            "same transaction"
        );

        // A commit statement ends it; the next statement starts a new one.
        assert!(!tx.statement_done(StatementType::Commit));
        assert!(tx.admit(StatementType::Select).unwrap());
        tx.read_only_started();

        // The commit and rollback calls do the same.
        tx.transaction_ended();
        assert!(tx.admit(StatementType::Select).unwrap());
        // ... but a commit statement itself needs no read-only start.
        assert!(!tx.admit(StatementType::Commit).unwrap());
        assert!(!tx.admit(StatementType::Rollback).unwrap());
    }

    #[test]
    fn back_to_auto_mode_stops_the_read_only_transaction_handling() {
        let mut tx = TxState::new(true);
        tx.set_mode(TxMode::Manual);
        assert!(tx.admit(StatementType::Select).unwrap());
        tx.set_mode(TxMode::Auto);
        assert!(!tx.admit(StatementType::Select).unwrap());
    }

    #[test]
    fn a_writable_connection_never_starts_read_only_transactions() {
        let mut tx = TxState::new(false);
        tx.set_mode(TxMode::Manual);
        tx.transaction_ended();
        assert!(!tx.admit(StatementType::Select).unwrap());
    }
}
