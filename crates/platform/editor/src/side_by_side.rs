//! What the editors of a side-by-side diff need (the reference's
//! `TwosideTextDiffViewer`): the mirrored layout of the left side, its line
//! numbers at the right next to the divider and its scrollbar at the left
//! (`EditorEx.setVerticalScrollbarOrientation(LEFT)`); no caret row and no
//! page below the text (`DiffUtil.createEditor`); and scrolling set by the
//! owner, which keeps the other side in step (`SyncScrollSupport`). The
//! changed words are [`crate::DiffHighlights`].

use gpui::{Context, Pixels, Point, px};

use crate::editor::Editor;

/// The diff editors' gutter, measured on the reference (New UI, 13 px line
/// numbers): from the line numbers' outer edge to the gutter's outer edge,
/// and from their inner edge to the text, where the column of the
/// changes' actions is. The numbers of a mirrored gutter are left-aligned,
/// nearer the text than on the right.
const OUTER: f32 = 6.4;
const NUMBERS_TO_TEXT: f32 = 46.0;
const MIRRORED_NUMBERS_TO_TEXT: f32 = 42.0;
/// One side alone has no action column: its text is this far after the
/// numbers.
const PLAIN_NUMBERS_TO_TEXT: f32 = 26.5;
/// The action column (`>>`, the check boxes: 14 px icons) ends this far
/// before the text; in a mirrored gutter it starts this far after it.
pub const ACTION_SIZE: f32 = 14.0;
const ACTION_TO_TEXT: f32 = 24.0;
const MIRRORED_ACTION_FROM_TEXT: f32 = 19.0;

/// The gutter of a diff's editor ([`Editor::set_diff_gutter`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffGutter {
    /// The editor's own gutter.
    #[default]
    Off,
    /// One side shown alone (`SimpleOnesideDiffViewer`): the numbers and
    /// the separator, no column of change actions.
    Plain,
    /// A side of a side-by-side diff: the numbers next to the divider and
    /// the column of the changes' actions.
    Actions,
}

/// Where a view scrolls once it is laid out, from its row height and its
/// height ([`Editor::scroll_on_layout`]).
pub type LayoutScroll = Box<dyn FnOnce(Pixels, Pixels) -> Option<Point<Pixels>>>;

/// The diff gutter's width for `digits` digits of `number_width` each,
/// with the action column or without.
pub(crate) fn diff_gutter_width(number_width: Pixels, digits: usize, actions: bool) -> Pixels {
    px(OUTER + numbers_to_text(actions)) + number_width * digits as f32
}

fn numbers_to_text(actions: bool) -> f32 {
    if actions {
        NUMBERS_TO_TEXT
    } else {
        PLAIN_NUMBERS_TO_TEXT
    }
}

/// Where the line numbers of a diff gutter go: the right edge (normal) or
/// the left edge (mirrored), from the gutter's left edge `gutter_x`.
pub(crate) fn diff_numbers_x(
    gutter_x: Pixels,
    gutter_width: Pixels,
    mirrored: bool,
    actions: bool,
) -> Pixels {
    if mirrored {
        gutter_x + px(MIRRORED_NUMBERS_TO_TEXT)
    } else {
        gutter_x + gutter_width - px(numbers_to_text(actions))
    }
}

/// The action column's left edge: before the text, or in a mirrored
/// editor after its end (`text_end`, the gutter's left edge).
pub(crate) fn diff_action_x(text_x: Pixels, text_end: Pixels, mirrored: bool) -> Pixels {
    if mirrored {
        text_end + px(MIRRORED_ACTION_FROM_TEXT)
    } else {
        text_x - px(ACTION_TO_TEXT + ACTION_SIZE)
    }
}

impl Editor {
    /// Lays the gutter out as the reference's diff editors have it: the
    /// line numbers at its outer side, a column for the changes' actions
    /// between them and the text, a separator line before the text, and
    /// the text right after it (no breakpoints, change markers or
    /// folding area).
    pub fn set_diff_gutter(&mut self, gutter: DiffGutter, cx: &mut Context<Self>) {
        if self.diff_gutter != gutter {
            self.diff_gutter = gutter;
            cx.notify();
        }
    }

    pub fn diff_gutter(&self) -> DiffGutter {
        self.diff_gutter
    }

    /// Where the diff gutter's action column (the left edge of its icons)
    /// was in the last frame, from the editor's left edge; `None` before
    /// the first frame or without a diff gutter. A title above the editor
    /// puts its check box there.
    pub fn diff_action_offset(&self) -> Option<Pixels> {
        let l = self
            .layout
            .as_ref()
            .filter(|_| self.diff_gutter == DiffGutter::Actions)?;
        Some(diff_action_x(l.text_x, l.text_end, self.mirrored) - l.bounds.left())
    }

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

    /// Scrolls the view where `f` says in its next layout, before its
    /// rows are laid out: the first scroll of a view that has not been
    /// drawn yet (its height unknown), in that very frame. Not reported as
    /// [`crate::EditorEvent::Scrolled`].
    pub fn scroll_on_layout(&mut self, f: LayoutScroll, cx: &mut Context<Self>) {
        self.layout_scroll = Some(f);
        cx.notify();
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
