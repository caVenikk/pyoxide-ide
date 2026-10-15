//! Undo and redo. Every change to a buffer is a step of replacements; steps
//! are grouped into transactions, and one undo reverts one transaction.
//! Consecutive typing (or deleting) merges into one transaction until the
//! caret moves, the kind of edit changes, the buffer is saved, or a pause
//! longer than [`GROUP_INTERVAL`] passes.

use std::time::{Duration, Instant};

use crate::selection::SelectionSet;

/// Longest pause between two edits of one undo group.
pub const GROUP_INTERVAL: Duration = Duration::from_secs(1);

/// What an edit was, for grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Typed text; consecutive typing is undone at once.
    Typing,
    /// Backspace.
    DeleteBackward,
    /// Delete.
    DeleteForward,
    /// Anything else (paste, cut, line operations): its own undo step.
    Other,
    /// The file's text from disk (Reload from Disk): its own undo step,
    /// which the reference asks about before undoing or redoing it.
    Reload,
}

impl EditKind {
    fn merges(self) -> bool {
        !matches!(self, EditKind::Other | EditKind::Reload)
    }
}

/// One replacement of a step. `start` is in the coordinates of the text
/// before the step; replacements of a step are sorted and do not overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub start: usize,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Transaction {
    pub id: u64,
    pub steps: Vec<Vec<Replacement>>,
    pub before: SelectionSet,
    pub after: SelectionSet,
    kind: EditKind,
    last_time: Instant,
    sealed: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    next_id: u64,
    /// Id of the newest transaction when the buffer was last saved (0: no
    /// transaction), `None` when that state can no longer be reached.
    saved: Option<u64>,
}

impl History {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            saved: Some(0),
            ..Self::default()
        }
    }

    fn top_id(&self) -> u64 {
        self.undo.last().map_or(0, |t| t.id)
    }

    pub fn record(
        &mut self,
        step: Vec<Replacement>,
        kind: EditKind,
        before: &SelectionSet,
        after: &SelectionSet,
        now: Instant,
    ) {
        if step.is_empty() {
            return;
        }
        if self
            .saved
            .is_some_and(|saved| self.redo.iter().any(|t| t.id == saved))
        {
            self.saved = None;
        }
        self.redo.clear();
        if let Some(last) = self.undo.last_mut()
            && kind.merges()
            && last.kind == kind
            && !last.sealed
            && now.saturating_duration_since(last.last_time) < GROUP_INTERVAL
            && last.after == *before
        {
            last.steps.push(step);
            last.after = after.clone();
            last.last_time = now;
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.undo.push(Transaction {
            id,
            steps: vec![step],
            before: before.clone(),
            after: after.clone(),
            kind,
            last_time: now,
            sealed: false,
        });
    }

    /// Ends the current undo group: the next edit starts a new one.
    pub fn seal(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.sealed = true;
        }
    }

    pub fn pop_undo(&mut self) -> Option<Transaction> {
        let t = self.undo.pop()?;
        Some(t)
    }

    pub fn push_redo(&mut self, mut t: Transaction) {
        t.sealed = true;
        self.redo.push(t);
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.redo.pop()
    }

    pub fn push_undo(&mut self, mut t: Transaction) {
        t.sealed = true;
        self.undo.push(t);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// What the next undo and redo would revert or apply again.
    pub fn undo_kind(&self) -> Option<EditKind> {
        self.undo.last().map(|t| t.kind)
    }

    pub fn redo_kind(&self) -> Option<EditKind> {
        self.redo.last().map(|t| t.kind)
    }

    /// The saved state can no longer be reached: the text differs from
    /// the file until the next save.
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Ends the current group and returns the id of the newest transaction.
    pub fn save_point(&mut self) -> u64 {
        self.seal();
        self.top_id()
    }

    /// The state with this newest transaction is what is on disk.
    pub fn mark_saved_at(&mut self, id: u64) {
        self.saved = Some(id);
    }

    pub fn mark_saved(&mut self) {
        self.seal();
        self.saved = Some(self.top_id());
    }

    pub fn is_modified(&self) -> bool {
        self.saved != Some(self.top_id())
    }
}
