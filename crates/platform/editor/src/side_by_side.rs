//! What the editors of a side-by-side diff need (the reference's
//! `TwosideTextDiffViewer`): the mirrored layout of the left side, its line
//! numbers at the right next to the divider and its scrollbar at the left
//! (`EditorEx.setVerticalScrollbarOrientation(LEFT)`); no caret row and no
//! page below the text (`DiffUtil.createEditor`); and scrolling set by the
//! owner, which keeps the other side in step (`SyncScrollSupport`). The
//! changed words are [`crate::DiffHighlights`].

use gpui::{Context, Pixels, Point, px};

use crate::editor::Editor;

impl Editor {
    /// Lays the editor out mirrored: the line numbers at the right edge,
    /// the scrollbar and the error stripe at the left.
    pub fn set_mirrored(&mut self, mirrored: bool, cx: &mut Context<Self>) {
        if self.mirrored != mirrored {
            self.mirrored = mirrored;
            cx.notify();
        }
    }

    pub fn is_mirrored(&self) -> bool {
        self.mirrored
    }

    /// Highlights the caret's row or not (the reference's diff editors do
    /// not, `DiffUtil.setEditorCodeStyle`).
    pub fn set_caret_row_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.caret_row_shown != shown {
            self.caret_row_shown = shown;
            cx.notify();
        }
    }

    /// The text scrolls until its last row is `rows` rows above the view's
    /// bottom (the reference's additional lines, without its page at the
    /// bottom); `None`: until the last row is at the view's top.
    pub fn set_room_after_end(&mut self, rows: Option<usize>, cx: &mut Context<Self>) {
        if self.room_after_end != rows {
            self.room_after_end = rows;
            cx.notify();
        }
    }

    /// Rows of room after the last one: a console has none, an editor
    /// with [`Self::set_room_after_end`] its own.
    pub(crate) fn lines_after_end(&self) -> Option<usize> {
        if self.console {
            Some(0)
        } else {
            self.room_after_end
        }
    }

    /// Scrolls the view to `position` (clamped to the text) without
    /// reporting [`crate::EditorEvent::Scrolled`]: the owner moves this view
    /// with another one.
    pub fn set_scroll_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let max_y = self.max_scroll_y();
        let y = position.y.clamp(px(0.0), max_y);
        let x = if self.soft_wraps() {
            px(0.0)
        } else {
            position.x.max(px(0.0))
        };
        if self.scroll.y != y || self.scroll.x != x {
            self.scroll.y = y;
            self.scroll.x = x;
            cx.notify();
        }
    }

    /// The scroll offset of the view.
    pub fn scroll_position(&self) -> Point<Pixels> {
        self.scroll
    }

    /// The height of a row and of the view in the last frame.
    pub fn view_metrics(&self) -> Option<(Pixels, Pixels)> {
        let l = self.layout.as_ref()?;
        Some((l.line_height, l.bounds.size.height))
    }

    /// How far the view scrolls down at most (the last row at the top).
    pub fn max_scroll_top(&self) -> Pixels {
        self.max_scroll_y()
    }
}
