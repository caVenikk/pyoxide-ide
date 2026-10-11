//! Transaction bookkeeping for drivers that track the open transaction
//! themselves instead of asking the backend.

use crate::TxMode;

/// A session's transaction mode and whether a transaction is open.
///
/// In [`TxMode::Auto`] every statement commits on its own. In
/// [`TxMode::Manual`] a transaction opens with the first statement that
/// needs one: implicitly on backends that start transactions themselves
/// (Oracle), or by a `BEGIN` the driver sends first ([`TxState::needs_begin`],
/// so an idle session holds no locks). It stays open until a commit or
/// rollback. Leaving manual mode commits what is pending, as JDBC's
/// auto-commit switch does ([`TxState::switch_commits`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TxState {
    mode: TxMode,
    open: bool,
}

impl TxState {
    pub fn mode(self) -> TxMode {
        self.mode
    }

    pub fn is_auto(self) -> bool {
        self.mode == TxMode::Auto
    }

    /// A transaction is open (changes may be pending).
    pub fn is_open(self) -> bool {
        self.open
    }

    /// Before a statement: whether the driver has to send `BEGIN` first.
    /// `statement_needs_tx` is `false` for statements that control the
    /// transaction themselves or must run outside one.
    pub fn needs_begin(self, statement_needs_tx: bool) -> bool {
        self.mode == TxMode::Manual && !self.open && statement_needs_tx
    }

    /// A transaction opened (the driver's `BEGIN`, the user's own, or a
    /// statement that opens one implicitly).
    pub fn begun(&mut self) {
        self.open = true;
    }

    /// The transaction ended (commit, rollback, or a statement that commits).
    pub fn ended(&mut self) {
        self.open = false;
    }

    /// Whether switching to `to` has to commit the open transaction first.
    pub fn switch_commits(self, to: TxMode) -> bool {
        self.mode == TxMode::Manual && to == TxMode::Auto && self.open
    }

    pub fn set_mode(&mut self, mode: TxMode) {
        self.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_mode_never_begins() {
        let tx = TxState::default();
        assert_eq!(tx.mode(), TxMode::Auto);
        assert!(tx.is_auto());
        assert!(!tx.needs_begin(true));
        assert!(!tx.switch_commits(TxMode::Manual));
    }

    #[test]
    fn manual_mode_begins_lazily_once() {
        let mut tx = TxState::default();
        tx.set_mode(TxMode::Manual);
        assert!(!tx.is_auto());
        assert!(tx.needs_begin(true));
        assert!(!tx.needs_begin(false), "COMMIT, BEGIN, VACUUM ...");
        tx.begun();
        assert!(tx.is_open());
        assert!(!tx.needs_begin(true), "already open");
        tx.ended();
        assert!(tx.needs_begin(true), "the next write opens a new one");
    }

    #[test]
    fn leaving_manual_mode_commits_only_what_is_open() {
        let mut tx = TxState::default();
        tx.set_mode(TxMode::Manual);
        assert!(!tx.switch_commits(TxMode::Auto), "nothing pending");
        tx.begun();
        assert!(tx.switch_commits(TxMode::Auto));
        assert!(!tx.switch_commits(TxMode::Manual));
    }
}
