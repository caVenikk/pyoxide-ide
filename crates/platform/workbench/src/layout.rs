//! Sizes of the main window's areas for a window size, as the reference
//! IDE's tool window pane lays them out (`ToolWindowPane`: two
//! `ThreeComponentsSplitter`s, one down with the editor row and the bottom
//! area, one across the row with the left area, the editor and the right
//! area).
//!
//! The editor gives way first: the tool window areas keep the sizes the user
//! gave them while the editor is larger than its minimum. Then the areas of a
//! splitter lose what is missing in proportion to their sizes, down to the
//! same minimum; below the minimums the last component runs past the far
//! edge, where it is cut. The sizes the user gave are not changed, so
//! growing the window back restores them.

use gpui::{App, Pixels, Size, px, size};
use themes::Metric;
use ui::theme::ThemeExt;

/// Minimum size of the editor area and of a tool window area in the main
/// window (the reference's registry key `ide.mainSplitter.min.size`).
pub const MAIN_SPLITTER_MIN_SIZE: f32 = 30.0;

/// Narrowest main window (the reference's `IdeProjectFrameAllocator` sets a
/// frame's minimum width to 340).
pub const WINDOW_MIN_WIDTH: f32 = 340.0;

/// The smallest island of an area. The reference counts the margins around
/// an island into its component, so an island there is the minimum less the
/// gap between two islands; here the gaps are the splitters between the
/// islands.
pub fn min_island(gap: f32) -> f32 {
    (MAIN_SPLITTER_MIN_SIZE - gap).max(0.0)
}

/// Smallest size of the main window: the minimum width, and the height of
/// the toolbar, the status bar and the editor row above the bottom area,
/// both at their minimum. The reference leaves the height to the system; at
/// this height both areas have reached their minimum.
pub fn window_min_size(cx: &App) -> Size<Pixels> {
    let theme = cx.theme();
    let gap = f32::from(theme.metric(Metric::IslandGap));
    let fixed = theme.metric(Metric::ToolbarHeight) + crate::chrome::status_metrics(cx).0;
    size(
        px(WINDOW_MIN_WIDTH),
        fixed + px(2.0 * min_island(gap) + gap),
    )
}

/// Sizes along one splitter: the first component, the inner one (the editor
/// area, or the row of the editor and the side areas) and the last one; a
/// component that is not shown has 0.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Split {
    pub first: f32,
    pub inner: f32,
    pub last: f32,
}

/// Lays out `first`, `inner` and `last` along `total` (the reference's
/// `ThreeComponentsSplitter.doLayout`). `first` and `last` are the sizes the
/// user gave the shown side components (`None`: not shown, and no divider
/// next to it); `divider` is the space between two shown components; every
/// component keeps `min`.
pub fn split(total: f32, first: Option<f32>, last: Option<f32>, divider: f32, min: f32) -> Split {
    let dividers = (u8::from(first.is_some()) + u8::from(last.is_some())) as f32 * divider;
    let space = total - dividers;
    if space <= 0.0 {
        return Split::default();
    }
    // A size is never below the minimum (the splitter's size setters).
    let mut f = first.map_or(0.0, |s| s.max(min));
    let mut l = last.map_or(0.0, |s| s.max(min));
    let lack = f + l - (space - min);
    if lack <= 0.0 {
        return Split {
            first: f,
            inner: space - f - l,
            last: l,
        };
    }
    // Both sides lose what is missing in proportion to their sizes; a side
    // at its minimum does not pass its share on to the other one.
    if f + l > 0.0 {
        let ratio = f / (f + l);
        if first.is_some() {
            f = (f - lack * ratio).max(min);
        }
        if last.is_some() {
            l = (l - lack * (1.0 - ratio)).max(min);
        }
    }
    Split {
        first: f,
        inner: min,
        last: l,
    }
}

/// The size dragging a side component's divider gives it (the reference's
/// `Divider.clamp`): `wanted` at least `min` and at most what leaves the
/// inner component its minimum next to the other side's size (`other`, the
/// size the user gave it; `None` when it is not shown). When the window is
/// too small for that the drag only keeps the minimum.
pub fn drag(wanted: f32, total: f32, other: Option<f32>, divider: f32, min: f32) -> f32 {
    let dividers = (1 + u8::from(other.is_some())) as f32 * divider;
    let max = total - other.unwrap_or(0.0) - min - dividers;
    let wanted = if min <= max {
        wanted.clamp(min, max)
    } else {
        wanted
    };
    wanted.max(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: f32 = 25.0;
    const GAP: f32 = 5.0;

    fn across(total: f32, left: Option<f32>, right: Option<f32>) -> Split {
        split(total, left, right, GAP, MIN)
    }

    #[test]
    fn the_editor_gives_way_first() {
        let s = across(1200.0, Some(333.0), Some(300.0));
        assert_eq!(s.first, 333.0);
        assert_eq!(s.last, 300.0);
        assert_eq!(s.inner, 1200.0 - 10.0 - 633.0);
        // Down to the editor's minimum the sides keep their sizes.
        let s = across(633.0 + 10.0 + MIN, Some(333.0), Some(300.0));
        assert_eq!((s.first, s.inner, s.last), (333.0, MIN, 300.0));
    }

    #[test]
    fn then_both_sides_shrink_in_proportion() {
        let s = across(633.0 + 10.0 + MIN - 63.3, Some(333.0), Some(300.0));
        assert_eq!(s.inner, MIN);
        assert!((s.first - (333.0 - 33.3)).abs() < 0.01, "{s:?}");
        assert!((s.last - (300.0 - 30.0)).abs() < 0.01, "{s:?}");
        assert!((s.first + s.inner + s.last + 10.0 - (633.0 + 10.0 + MIN - 63.3)).abs() < 0.01);
    }

    #[test]
    fn one_side_alone_gives_way_after_the_editor() {
        let s = across(400.0, Some(333.0), None);
        assert_eq!(
            (s.first, s.inner, s.last),
            (333.0, 400.0 - 5.0 - 333.0, 0.0)
        );
        let s = across(300.0, Some(333.0), None);
        assert_eq!((s.first, s.inner, s.last), (300.0 - 5.0 - MIN, MIN, 0.0));
        let s = across(300.0, None, Some(333.0));
        assert_eq!((s.first, s.inner, s.last), (0.0, MIN, 300.0 - 5.0 - MIN));
    }

    #[test]
    fn below_the_minimums_the_last_component_runs_past_the_end() {
        let s = across(50.0, Some(333.0), Some(300.0));
        assert_eq!((s.first, s.inner, s.last), (MIN, MIN, MIN));
        // Only the editor: it keeps its minimum.
        let s = across(10.0, None, None);
        assert_eq!(s.inner, MIN);
        // No room for the dividers: nothing is shown.
        assert_eq!(across(8.0, Some(333.0), Some(300.0)), Split::default());
    }

    #[test]
    fn a_side_at_its_minimum_keeps_its_share() {
        // The right side at its minimum: the left one loses only its own
        // share, as in the reference; the rest runs past the far edge.
        let total = 333.0 + MIN + 10.0 + MIN - 100.0;
        let s = across(total, Some(333.0), Some(MIN));
        assert_eq!(s.last, MIN);
        let share = 100.0 * 333.0 / (333.0 + MIN);
        assert!((s.first - (333.0 - share)).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn sizes_below_the_minimum_count_as_the_minimum() {
        let s = across(1000.0, Some(3.0), None);
        assert_eq!(s.first, MIN);
    }

    #[test]
    fn dragging_keeps_the_minimums() {
        // Left divider, right side shown with 300.
        assert_eq!(drag(500.0, 1200.0, Some(300.0), GAP, MIN), 500.0);
        assert_eq!(drag(0.0, 1200.0, Some(300.0), GAP, MIN), MIN);
        assert_eq!(
            drag(2000.0, 1200.0, Some(300.0), GAP, MIN),
            1200.0 - 300.0 - MIN - 10.0
        );
        // Without the other side.
        assert_eq!(drag(2000.0, 1200.0, None, GAP, MIN), 1200.0 - MIN - 5.0);
        // A window too small for the minimums: the drag keeps the minimum
        // only.
        assert_eq!(drag(80.0, 100.0, Some(300.0), GAP, MIN), 80.0);
        assert_eq!(drag(10.0, 100.0, Some(300.0), GAP, MIN), MIN);
    }

    #[test]
    fn island_minimum() {
        assert_eq!(min_island(5.0), 25.0);
        assert_eq!(min_island(40.0), 0.0);
    }
}
