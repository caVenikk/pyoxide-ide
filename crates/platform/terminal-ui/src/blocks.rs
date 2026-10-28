//! Command blocks of a session with shell integration (the reference's
//! `TerminalBlocksModel` and `TerminalBlocksDecorator`, `plugins/terminal`,
//! IntelliJ Community, Apache-2.0): the prompt mark (OSC 133;A) starts a
//! block, the next prompt ends it, and the finish mark (OSC 133;D) gives
//! its exit code. With "Show separators between executed commands" on,
//! every block after the first has a 1 px line above it, 6 px of space
//! around the line, and a rounded red marker beside a block that failed.
//!
//! The model works in absolute lines (lines of scrollback plus the row), so
//! the blocks stay where they are while the output scrolls. The view lays
//! out rows with room for the separators ([`RowLayout`]): the space
//! pushes the rows below it down, and when the screen is full the top
//! rows go under the edge.

use terminal::{CellFlags, MarkKind, Snapshot};

/// Space above a block's first row, the line included: the previous
/// block's bottom inset, the line, the top inset
/// (`TerminalUi.blockBottomInset`, 1, `blockTopInset`).
pub const BLOCK_GAP: f32 = 13.0;
/// From a block's first row up to the line (`blockTopInset` and the line).
pub const SEPARATOR_ABOVE: f32 = 7.0;
/// The line stops this far short of the right edge, except above the
/// prompt (`blockSeparatorRightOffset`).
pub const SEPARATOR_RIGHT_OFFSET: f32 = 19.0;
/// The failed-command marker: width, how far its top and bottom are
/// inside the block's rows, and corner radius (`errorLineWidth`,
/// `errorLineYOffset`, `errorLineArc` is the diameter).
pub const ERROR_WIDTH: f32 = 3.0;
pub const ERROR_Y_OFFSET: f32 = 2.0;
pub const ERROR_RADIUS: f32 = 2.0;
/// Where the marker's left edge is, from the left edge of the view
/// (`errorLineRightOffset` before the text at `LEFT_INSET`).
pub const ERROR_LEFT: f32 = 3.0;
/// The most blocks kept.
const MAX_BLOCKS: usize = 2000;

/// One command block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// The absolute line of its first row (the prompt's).
    pub start: u64,
    /// The exit code the finish mark gave.
    pub exit_code: Option<i32>,
}

impl Block {
    /// The marker beside the block: the command failed.
    pub fn failed(&self) -> bool {
        self.exit_code.is_some_and(|c| c != 0)
    }
}

/// The blocks of a session.
#[derive(Clone, Debug, Default)]
pub struct Blocks {
    list: Vec<Block>,
    /// Lines in the scrollback at the last mark.
    history: usize,
    /// Older blocks were dropped: the first one is not the first of the
    /// session.
    trimmed: bool,
    /// The scrollback is full: lines drop off its top without a trace, so
    /// the blocks' lines cannot be followed any more.
    lost: bool,
}

/// A block as the visible screen shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleBlock {
    /// The row of its first line, when it is on the screen.
    pub start_row: Option<usize>,
    /// Its last row on the screen.
    pub end_row: usize,
    /// There is room and a line above its first row: not the first block of
    /// the session, and its first row is not the top one.
    pub separated: bool,
    pub failed: bool,
    /// The block of the prompt in use (no room under it).
    pub current: bool,
}

impl Blocks {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.list
    }

    /// A mark of the shell integration. `line` is the absolute line the
    /// cursor was on, `history` the lines in the scrollback, `capacity`
    /// how many it keeps.
    pub fn mark(&mut self, kind: MarkKind, line: u64, history: usize, capacity: usize) {
        if history < self.history {
            // The scrollback was cleared: the lines moved.
            self.list.clear();
            self.trimmed = false;
        }
        self.history = history;
        if capacity > 0 && history >= capacity {
            self.lost = true;
            self.list.clear();
            return;
        }
        self.lost = false;
        match kind {
            MarkKind::PromptStart => {
                // A prompt drawn again, or a screen cleared under us.
                while self.list.last().is_some_and(|b| b.start >= line) {
                    self.list.pop();
                }
                self.list.push(Block {
                    start: line,
                    exit_code: None,
                });
                if self.list.len() > MAX_BLOCKS {
                    self.list.remove(0);
                    self.trimmed = true;
                }
            }
            MarkKind::CommandFinished { exit_code } => {
                if let Some(last) = self.list.last_mut() {
                    last.exit_code = exit_code;
                }
            }
            MarkKind::CommandStart | MarkKind::CommandExecuted => {}
        }
    }

    /// Clear Terminal moved the prompt's rows to the top and dropped
    /// everything above them: only the block of the prompt is left, at the
    /// top.
    pub fn cleared(&mut self) {
        let last = self.list.last().copied();
        self.list.clear();
        self.trimmed = false;
        self.history = 0;
        if let Some(mut last) = last {
            last.start = 0;
            self.list.push(last);
        }
    }

    /// The screen was resized to another width: lines were joined or split
    /// and the blocks' lines no longer fit.
    pub fn reflowed(&mut self) {
        self.list.clear();
        self.trimmed = false;
    }

    /// The blocks on a screen of `rows` rows whose first row is the
    /// absolute line `top`, oldest first; the first one may start above it.
    pub fn visible(&self, top: u64, rows: usize) -> Vec<VisibleBlock> {
        let mut out = Vec::new();
        if rows == 0 || self.list.is_empty() {
            return out;
        }
        let bottom = top + rows as u64;
        // The block that holds the top row is the last one that starts at
        // or above it.
        let first = self
            .list
            .partition_point(|b| b.start <= top)
            .saturating_sub(1);
        for (i, block) in self.list.iter().enumerate().skip(first) {
            if block.start >= bottom {
                break;
            }
            let next = self.list.get(i + 1).map(|n| n.start);
            let end = next.map_or(bottom, |n| n.min(bottom));
            if end <= top {
                continue;
            }
            let start_row = (block.start >= top).then(|| (block.start - top) as usize);
            let session_first = i == 0 && !self.trimmed;
            out.push(VisibleBlock {
                start_row,
                end_row: (end - top - 1) as usize,
                separated: !session_first && start_row.is_some_and(|r| r >= 1),
                failed: block.failed(),
                current: next.is_none(),
            });
        }
        out
    }
}

/// Where the rows of the screen are: their number, the room the
/// separators take above some of them, and how far the rows are pushed up
/// when they do not fit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowLayout {
    /// Room above each row, all rows above it included; one more than the
    /// rows.
    above: Vec<f32>,
    /// The rows are shifted up by this much (the screen is full).
    shift: f32,
}

impl RowLayout {
    /// Rows without room between them.
    pub fn plain(rows: usize) -> Self {
        Self {
            above: vec![0.0; rows + 1],
            shift: 0.0,
        }
    }

    /// The layout of `rows` rows with a gap above each row in `gaps`
    /// (ascending). The content ends at `last_row`; when it ends below
    /// `height` the rows go up by the difference.
    pub fn new(
        rows: usize,
        gaps: impl IntoIterator<Item = usize>,
        cell_height: f32,
        last_row: usize,
        height: f32,
    ) -> Self {
        let mut above = vec![0.0; rows + 1];
        for row in gaps {
            if row < above.len() {
                for a in &mut above[row..] {
                    *a += BLOCK_GAP;
                }
            }
        }
        let last = last_row.min(rows.saturating_sub(1));
        let bottom = (last + 1) as f32 * cell_height + above[last];
        Self {
            above,
            shift: (bottom - height).max(0.0),
        }
    }

    /// No room anywhere.
    pub fn is_plain(&self) -> bool {
        self.shift == 0.0 && self.above.last().is_none_or(|a| *a == 0.0)
    }

    /// The top of a row from the top of the grid.
    pub fn top(&self, row: usize, cell_height: f32) -> f32 {
        let extra = self
            .above
            .get(row)
            .or(self.above.last())
            .copied()
            .unwrap_or(0.0);
        row as f32 * cell_height + extra - self.shift
    }

    /// The row at a height from the top of the grid, clamped to the
    /// grid; the room above a row belongs to it.
    pub fn row_at(&self, y: f32, rows: usize, cell_height: f32) -> usize {
        if rows == 0 {
            return 0;
        }
        if self.is_plain() {
            return ((y / cell_height).max(0.0) as usize).min(rows - 1);
        }
        let y = y + self.shift;
        // The last row whose room starts at or above `y`.
        let (mut low, mut high) = (0, rows - 1);
        while low < high {
            let mid = (low + high).div_ceil(2);
            let room_top = mid as f32 * cell_height + self.above[mid] - self.gap_above(mid);
            if room_top <= y {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        low
    }

    /// Height of the room right above a row.
    pub fn gap_above(&self, row: usize) -> f32 {
        let here = self.above.get(row).copied().unwrap_or(0.0);
        let before = row
            .checked_sub(1)
            .and_then(|r| self.above.get(r))
            .copied()
            .unwrap_or(0.0);
        here - before
    }

    /// How far the rows are pushed up.
    pub fn shift(&self) -> f32 {
        self.shift
    }
}

/// The last row of the screen with something on it: text, or the cursor.
pub fn last_content_row(snapshot: &Snapshot) -> usize {
    let cursor = snapshot.cursor.map_or(0, |c| c.point.line);
    for line in (cursor.min(snapshot.rows.saturating_sub(1)) + 1..snapshot.rows).rev() {
        let blank = snapshot
            .row(line)
            .iter()
            .all(|c| (c.c == ' ' || c.c == '\0') && !c.flags.contains(CellFlags::INVERSE));
        if !blank {
            return line;
        }
    }
    cursor.min(snapshot.rows.saturating_sub(1))
}

/// The first row from `from` to `to` (inclusive) with text in it: the
/// marker of a failed block starts there, below the blank line some
/// prompts print first.
pub fn first_text_row(snapshot: &Snapshot, from: usize, to: usize) -> usize {
    (from..=to.min(snapshot.rows.saturating_sub(1)))
        .find(|&line| snapshot.row(line).iter().any(|c| c.c != ' ' && c.c != '\0'))
        .unwrap_or(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(blocks: &mut Blocks, line: u64) {
        blocks.mark(MarkKind::PromptStart, line, 0, 10_000);
    }

    fn finished(blocks: &mut Blocks, code: i32) {
        blocks.mark(
            MarkKind::CommandFinished {
                exit_code: Some(code),
            },
            0,
            0,
            10_000,
        );
    }

    #[test]
    fn prompts_start_blocks_and_finish_marks_give_exit_codes() {
        let mut b = Blocks::default();
        prompt(&mut b, 0);
        finished(&mut b, 0);
        prompt(&mut b, 3);
        finished(&mut b, 1);
        prompt(&mut b, 5);
        assert_eq!(
            b.blocks(),
            [
                Block {
                    start: 0,
                    exit_code: Some(0)
                },
                Block {
                    start: 3,
                    exit_code: Some(1)
                },
                Block {
                    start: 5,
                    exit_code: None
                },
            ]
        );
        assert!(!b.blocks()[0].failed() && b.blocks()[1].failed());
    }

    #[test]
    fn a_prompt_drawn_again_replaces_its_block() {
        let mut b = Blocks::default();
        prompt(&mut b, 4);
        prompt(&mut b, 4);
        prompt(&mut b, 6);
        prompt(&mut b, 5);
        assert_eq!(b.len(), 2);
        assert_eq!(b.blocks()[1].start, 5);
    }

    #[test]
    fn visible_blocks_have_rows_separators_and_markers() {
        let mut b = Blocks::default();
        prompt(&mut b, 0);
        finished(&mut b, 0);
        prompt(&mut b, 3);
        finished(&mut b, 2);
        prompt(&mut b, 7);
        let v = b.visible(0, 10);
        assert_eq!(v.len(), 3);
        // The first block has no room above it.
        assert_eq!(
            (v[0].start_row, v[0].end_row, v[0].separated),
            (Some(0), 2, false)
        );
        assert_eq!(
            (v[1].start_row, v[1].end_row, v[1].separated),
            (Some(3), 6, true)
        );
        assert!(v[1].failed && !v[1].current);
        assert_eq!((v[2].start_row, v[2].end_row), (Some(7), 9));
        assert!(v[2].current && !v[2].failed);
        // Scrolled: the middle block starts above the screen.
        let v = b.visible(5, 5);
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].start_row, v[0].end_row, v[0].separated),
            (None, 1, false)
        );
        assert_eq!(v[1].start_row, Some(2));
        // Far below everything: only the last block.
        let v = b.visible(100, 5);
        assert_eq!(v.len(), 1);
        assert!(v[0].current);
        // Above everything.
        assert!(b.visible(0, 0).is_empty());
    }

    #[test]
    fn the_block_at_the_top_row_has_no_room_above() {
        let mut b = Blocks::default();
        prompt(&mut b, 0);
        prompt(&mut b, 4);
        let v = b.visible(4, 5);
        assert_eq!(v[0].start_row, Some(0));
        assert!(!v[0].separated);
    }

    #[test]
    fn clearing_history_or_reflowing_forgets_the_blocks() {
        let mut b = Blocks::default();
        b.mark(MarkKind::PromptStart, 10, 8, 100);
        b.mark(MarkKind::PromptStart, 15, 12, 100);
        assert_eq!(b.len(), 2);
        // History shrank: the lines moved.
        b.mark(MarkKind::PromptStart, 2, 0, 100);
        assert_eq!(b.len(), 1);
        b.reflowed();
        assert!(b.is_empty());
        b.mark(MarkKind::PromptStart, 3, 0, 100);
        b.mark(MarkKind::CommandFinished { exit_code: Some(1) }, 3, 0, 100);
        b.cleared();
        assert_eq!(
            b.blocks(),
            [Block {
                start: 0,
                exit_code: Some(1)
            }]
        );
    }

    #[test]
    fn a_full_scrollback_loses_track() {
        let mut b = Blocks::default();
        b.mark(MarkKind::PromptStart, 5, 0, 100);
        b.mark(MarkKind::PromptStart, 105, 100, 100);
        assert!(b.is_empty());
        // After a clear the blocks start again.
        b.mark(MarkKind::PromptStart, 0, 3, 100);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn rows_get_room_and_the_screen_shifts_up_when_full() {
        let l = RowLayout::new(10, [3, 7], 10.0, 9, 1000.0);
        assert_eq!(l.top(0, 10.0), 0.0);
        assert_eq!(l.top(3, 10.0), 30.0 + BLOCK_GAP);
        assert_eq!(l.top(7, 10.0), 70.0 + 2.0 * BLOCK_GAP);
        assert_eq!(l.shift(), 0.0);
        // Content to row 9 needs 100 + 26 = 126; only 100 fit.
        let l = RowLayout::new(10, [3, 7], 10.0, 9, 100.0);
        assert_eq!(l.shift(), 26.0);
        assert_eq!(l.top(9, 10.0) + 10.0, 100.0);
        // The unused rows below the cursor do not count.
        let l = RowLayout::new(10, [3, 7], 10.0, 4, 100.0);
        assert_eq!(l.shift(), 0.0);
    }

    #[test]
    fn a_position_finds_its_row_through_the_room() {
        let l = RowLayout::new(10, [3, 7], 10.0, 4, 1000.0);
        assert_eq!(l.row_at(-5.0, 10, 10.0), 0);
        assert_eq!(l.row_at(25.0, 10, 10.0), 2);
        // The room above row 3 belongs to it.
        assert_eq!(l.row_at(30.0 + 2.0, 10, 10.0), 3);
        assert_eq!(l.row_at(30.0 + BLOCK_GAP + 4.0, 10, 10.0), 3);
        assert_eq!(l.row_at(30.0 + BLOCK_GAP + 10.0, 10, 10.0), 4);
        assert_eq!(l.row_at(5000.0, 10, 10.0), 9);
        let plain = RowLayout::plain(10);
        assert_eq!(plain.row_at(35.0, 10, 10.0), 3);
        assert!(plain.is_plain());
        assert!(!l.is_plain());
    }
}
