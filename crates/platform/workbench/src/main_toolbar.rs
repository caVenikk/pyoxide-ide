//! Layout of the main toolbar when space runs out, as the New UI lays it
//! out (`MainToolbar`: a `HorizontalLayout` of a left, a centre and a right
//! group, the widths shared out by `CompressingLayoutStrategy`):
//!
//! 1. The widgets with a text (project, file, run configuration) give way
//!    first, down to their minimum: the widest one shrinks until it is as
//!    narrow as the next one, then both shrink together, and so on. A text
//!    loses characters and gets an ellipsis ([`Cut`]).
//! 2. With every text at its minimum the groups keep their widths, and the
//!    right group runs past the window's right edge, cut there.
//!
//! The centre group is centred in the toolbar while it fits between the
//! other two, otherwise it keeps two gaps from the right group (or from the
//! left one). Nothing ever overlaps.

use gpui::{
    AnyElement, FontWeight, IntoElement, Pixels, SharedString, Window, div, prelude::*, px,
};

/// Gap of the toolbar's layout (`MainToolbar.layoutGap`): two of them lie
/// between groups, four are kept out of the room shared by the widgets.
pub const GAP: f32 = 10.0;

/// How a widget's text is cut when the widget is narrower than the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Characters go from the end and "..." follows; at least 4 stay
    /// (`DefaultCutStrategy`, `UIUtil.computeTextComponentMinimumSize`):
    /// the project and the file widgets.
    End,
    /// Cut in the middle: up to 8 characters of the end stay, "…" stands
    /// in between, at least 5 characters stay (`trimRunConfigurationName`,
    /// `RedesignedRunConfigurationSelectorButton.getMinimumSize`): the run
    /// configuration.
    Middle,
}

impl Cut {
    /// Characters a widget keeps at its narrowest.
    fn keep(self) -> usize {
        match self {
            Cut::End => 4,
            Cut::Middle => 5,
        }
    }

    pub fn ellipsis(self) -> &'static str {
        match self {
            Cut::End => "...",
            Cut::Middle => "\u{2026}",
        }
    }
}

/// Characters kept at the beginning and at the end of a run configuration's
/// name cut to `count` characters (`splitCharCount`): the end keeps 8 when
/// the beginning can keep 8 too, the end goes first, then the beginning.
fn split_count(count: usize) -> (usize, usize) {
    match count {
        c if c >= 16 => (c - 8, 8),
        c if c > 8 => (8, c - 8),
        c => (c, 0),
    }
}

/// A widget's text and the x of every character boundary, measured once.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    text: String,
    /// Byte offset of every character boundary, the end included.
    offsets: Vec<usize>,
    /// x of every boundary; the last one is the text's width.
    x: Vec<f32>,
    /// Width of the cut's ellipsis.
    ellipsis: f32,
    cut: Cut,
    /// The longest text shown, in characters (the run configuration's name
    /// is cut in the middle beyond it even with room to spare).
    max_chars: Option<usize>,
}

impl Measured {
    /// `x` holds the x of every character boundary of `text`, the end
    /// included.
    pub fn new(text: impl Into<String>, x: Vec<f32>, ellipsis: f32, cut: Cut) -> Self {
        let text = text.into();
        let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        offsets.push(text.len());
        assert_eq!(offsets.len(), x.len(), "one x per character boundary");
        Self {
            text,
            offsets,
            x,
            ellipsis,
            cut,
            max_chars: None,
        }
    }

    /// Every character `char_width` wide (tests).
    pub fn monospace(text: &str, char_width: f32, cut: Cut) -> Self {
        let n = text.chars().count();
        let x = (0..=n).map(|i| i as f32 * char_width).collect();
        let ellipsis = cut.ellipsis().chars().count() as f32 * char_width;
        Self::new(text, x, ellipsis, cut)
    }

    /// Shows at most `chars` characters, cut in the middle beyond them.
    pub fn max_chars(mut self, chars: usize) -> Self {
        self.max_chars = Some(chars);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn chars(&self) -> usize {
        self.x.len() - 1
    }

    fn full_width(&self) -> f32 {
        self.x[self.chars()]
    }

    /// Width of the text cut to `count` characters by [`Cut::Middle`].
    fn middle_width(&self, count: usize) -> f32 {
        let n = self.chars();
        let (prefix, suffix) = split_count(count);
        if prefix + suffix >= n {
            return self.full_width();
        }
        self.x[prefix] + self.ellipsis + self.full_width() - self.x[n - suffix]
    }

    fn middle_text(&self, count: usize) -> String {
        let n = self.chars();
        let (prefix, suffix) = split_count(count);
        if prefix + suffix >= n {
            return self.text.clone();
        }
        format!(
            "{}{}{}",
            &self.text[..self.offsets[prefix]],
            Cut::Middle.ellipsis(),
            &self.text[self.offsets[n - suffix]..]
        )
    }

    /// The widest the text gets.
    pub fn width(&self) -> f32 {
        match self.max_chars {
            Some(max) if self.chars() > max => self.middle_width(max - 1),
            _ => self.full_width(),
        }
    }

    /// The narrowest the text gets: its first characters and the
    /// ellipsis, never more than [`Self::width`].
    pub fn min_width(&self) -> f32 {
        let keep = self.cut.keep();
        if self.chars() <= keep {
            return self.width();
        }
        (self.x[keep] + self.ellipsis).min(self.width())
    }

    /// The text shown in `width` and whether it is cut.
    pub fn fit(&self, width: f32) -> (String, bool) {
        let n = self.chars();
        let capped = self.max_chars.is_some_and(|max| n > max);
        if self.full_width() <= width && !capped {
            return (self.text.clone(), false);
        }
        match self.cut {
            Cut::End => {
                if n <= Cut::End.keep() {
                    return (self.text.clone(), false);
                }
                let mut keep = n - 1;
                while keep > Cut::End.keep() && self.x[keep] + self.ellipsis > width {
                    keep -= 1;
                }
                let text = format!(
                    "{}{}",
                    &self.text[..self.offsets[keep]],
                    Cut::End.ellipsis()
                );
                (text, true)
            }
            Cut::Middle => {
                // The longest cut that fits, beyond `max_chars` never.
                let longest = self.max_chars.map_or(n - 1, |m| (m - 1).min(n - 1));
                let count = (0..=longest)
                    .rev()
                    .find(|&c| self.middle_width(c) <= width)
                    .unwrap_or(0);
                (self.middle_text(count), true)
            }
        }
    }
}

/// Measures texts in the UI font.
pub struct Measure<'a> {
    window: &'a Window,
    family: SharedString,
    size: Pixels,
}

impl<'a> Measure<'a> {
    pub fn new(window: &'a Window, family: SharedString, size: Pixels) -> Self {
        Self {
            window,
            family,
            size,
        }
    }

    fn line(&self, text: &str, weight: FontWeight) -> gpui::ShapedLine {
        let mut font = gpui::font(self.family.clone());
        font.weight = weight;
        let run = gpui::TextRun {
            len: text.len(),
            font,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.size,
            &[run],
            None,
        )
    }

    /// `text` in `weight`, to be cut by `cut`.
    pub fn text(&self, text: &str, weight: FontWeight, cut: Cut) -> Measured {
        let mut x = Vec::with_capacity(text.len() + 1);
        if text.is_empty() {
            x.push(0.0);
        } else {
            let line = self.line(text, weight);
            let width = f32::from(line.width);
            let mut glyphs = line
                .runs
                .iter()
                .flat_map(|run| run.glyphs.iter())
                .map(|g| (g.index, f32::from(g.position.x)))
                .peekable();
            for (offset, _) in text.char_indices() {
                while glyphs.next_if(|&(index, _)| index < offset).is_some() {}
                x.push(glyphs.peek().map_or(width, |&(_, x)| x));
            }
            x.push(width);
        }
        let ellipsis = f32::from(self.line(cut.ellipsis(), weight).width);
        Measured::new(text, x, ellipsis, cut)
    }
}

/// The width a widget wants and the least it takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub min: f32,
    pub pref: f32,
}

impl Extent {
    pub fn fixed(width: f32) -> Self {
        Self {
            min: width,
            pref: width,
        }
    }

    fn resizable(&self) -> bool {
        self.min < self.pref
    }
}

/// Widths of `items` in `available` (`CompressingLayoutStrategy`): the
/// preferred ones when they fit; otherwise the widgets that can give way
/// share what the others leave, the narrowest growing first, so the widest
/// ones are the first to shrink; never below their minimum.
pub fn distribute(available: f32, items: &[Extent]) -> Vec<f32> {
    let fixed: f32 = items
        .iter()
        .filter(|e| !e.resizable())
        .map(|e| e.pref)
        .sum();
    let room = available - fixed;
    let resizable = || items.iter().filter(|e| e.resizable());
    let level = if room >= resizable().map(|e| e.pref).sum::<f32>() {
        f32::INFINITY
    } else if room <= resizable().map(|e| e.min).sum::<f32>() {
        f32::NEG_INFINITY
    } else {
        // The water level: every resizable widget is the level wide,
        // within its minimum and its preferred width.
        let total =
            |level: f32| -> f32 { resizable().map(|e| level.clamp(e.min, e.pref)).sum::<f32>() };
        let mut breaks: Vec<f32> = resizable().flat_map(|e| [e.min, e.pref]).collect();
        breaks.sort_by(f32::total_cmp);
        let mut level = breaks[0];
        for pair in breaks.windows(2) {
            let (low, high) = (pair[0], pair[1]);
            if total(high) >= room {
                let growing = resizable().filter(|e| e.min <= low && e.pref > low).count();
                level = if growing == 0 {
                    low
                } else {
                    low + (room - total(low)) / growing as f32
                };
                break;
            }
        }
        level
    };
    items
        .iter()
        .map(|e| {
            if e.resizable() {
                level.clamp(e.min, e.pref)
            } else {
                e.pref
            }
        })
        .collect()
}

/// Where the groups of the toolbar go and how wide their widgets are.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// x of the left, centre and right groups from the toolbar's inner
    /// left edge.
    pub x: [f32; 3],
    /// Widths of the widgets of each group.
    pub widths: [Vec<f32>; 3],
}

impl Placement {
    /// Width of a group.
    pub fn width(&self, group: usize) -> f32 {
        self.widths[group].iter().sum()
    }
}

/// Places the left, centre and right groups (`HorizontalLayout`) of
/// widgets in a toolbar `width` wide.
pub fn place(width: f32, groups: [&[Extent]; 3]) -> Placement {
    let all: Vec<Extent> = groups.iter().flat_map(|g| g.iter().copied()).collect();
    let mut flat = distribute(width - 4.0 * GAP, &all).into_iter();
    let widths = groups.map(|g| flat.by_ref().take(g.len()).collect::<Vec<f32>>());
    let sum = |i: usize| widths[i].iter().sum::<f32>();
    let [left, centre, right] = groups.map(|g| !g.is_empty());
    let after_left = if left { sum(0) + 2.0 * GAP } else { 0.0 };
    let mut right_x = if right { width - sum(2) } else { width };
    right_x = right_x.max(after_left);
    let mut centre_x = 0.0;
    if centre {
        let w = sum(1);
        centre_x = (width - w) / 2.0;
        if centre_x > after_left && centre_x + w + 2.0 * GAP > right_x {
            centre_x = right_x - w - 2.0 * GAP;
        }
        centre_x = centre_x.max(after_left);
        right_x = right_x.max(centre_x + w + 2.0 * GAP);
    }
    Placement {
        x: [0.0, centre_x, right_x],
        widths,
    }
}

/// A widget's text as laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    pub text: SharedString,
    /// Width of the text's box.
    pub width: Pixels,
    /// The text is cut short.
    pub cut: bool,
}

type Draw<'a> = Box<dyn FnOnce(Pixels) -> AnyElement + 'a>;

/// A widget of the main toolbar: the width it wants, the least it takes
/// and how it is drawn at the width it gets.
pub struct Widget<'a> {
    extent: Extent,
    draw: Draw<'a>,
}

impl<'a> Widget<'a> {
    /// A widget that keeps its width.
    pub fn fixed(width: Pixels, element: impl IntoElement + 'a) -> Self {
        Self {
            extent: Extent::fixed(width.into()),
            draw: Box::new(move |_| element.into_any_element()),
        }
    }

    /// Empty space.
    pub fn space(width: f32) -> Self {
        Self::fixed(px(width), div().flex_none().w(px(width)))
    }

    /// A widget with a text that gives way: `chrome` is its width without
    /// the text; `draw` gets the text as it fits.
    pub fn text(chrome: f32, text: Measured, draw: impl FnOnce(Fitted) -> AnyElement + 'a) -> Self {
        Self {
            extent: Extent {
                min: chrome + text.min_width(),
                pref: chrome + text.width(),
            },
            draw: Box::new(move |width| {
                let room = (f32::from(width) - chrome).max(0.0);
                let (shown, cut) = text.fit(room);
                draw(Fitted {
                    text: shown.into(),
                    width: px(room),
                    cut,
                })
            }),
        }
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }
}

/// The groups of the toolbar placed in `width` from `left`: absolutely
/// positioned rows of widgets, each widget at the width it got.
pub fn render(left: Pixels, width: Pixels, groups: [Vec<Widget<'_>>; 3]) -> Vec<AnyElement> {
    let extents = groups
        .each_ref()
        .map(|g| g.iter().map(Widget::extent).collect::<Vec<_>>());
    let placement = place(
        width.into(),
        [&extents[0][..], &extents[1][..], &extents[2][..]],
    );
    let ids = ["toolbar-left", "toolbar-centre", "toolbar-right"];
    groups
        .into_iter()
        .enumerate()
        .filter(|(_, widgets)| !widgets.is_empty())
        .map(|(i, widgets)| {
            let id = ids[i];
            div()
                .id(id)
                .debug_selector(move || id.into())
                .absolute()
                .top_0()
                .bottom_0()
                .left(left + px(placement.x[i]))
                .flex()
                .flex_row()
                .items_center()
                .children(
                    widgets
                        .into_iter()
                        .zip(&placement.widths[i])
                        .map(|(w, &width)| (w.draw)(px(width))),
                )
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAR: f32 = 7.0;

    fn mono(text: &str, cut: Cut) -> Measured {
        Measured::monospace(text, CHAR, cut)
    }

    #[test]
    fn the_end_is_cut_with_three_dots_and_four_characters_stay() {
        let m = mono("skin-tech-api", Cut::End);
        assert_eq!(m.width(), 13.0 * CHAR);
        assert_eq!(m.min_width(), 7.0 * CHAR);
        assert_eq!(m.fit(13.0 * CHAR), ("skin-tech-api".into(), false));
        assert_eq!(m.fit(12.9 * CHAR), ("skin-tech...".into(), true));
        assert_eq!(m.fit(10.0 * CHAR), ("skin-te...".into(), true));
        assert_eq!(m.fit(7.0 * CHAR), ("skin...".into(), true));
        // Below the minimum the four characters still stay.
        assert_eq!(m.fit(2.0 * CHAR), ("skin...".into(), true));
    }

    #[test]
    fn short_texts_are_never_cut() {
        let m = mono("demo", Cut::End);
        assert_eq!(m.min_width(), m.width());
        assert_eq!(m.fit(CHAR), ("demo".into(), false));
        // "stab..." would be wider than "stable".
        let m = mono("stable", Cut::End);
        assert_eq!(m.min_width(), m.width());
    }

    #[test]
    fn the_run_configuration_keeps_five_characters_and_a_single_ellipsis() {
        let m = mono("Current File", Cut::Middle);
        assert_eq!(m.min_width(), 6.0 * CHAR);
        assert_eq!(m.fit(6.0 * CHAR), ("Curre\u{2026}".into(), true));
        assert_eq!(m.fit(9.0 * CHAR), ("Current \u{2026}".into(), true));
        assert_eq!(m.fit(12.0 * CHAR), ("Current File".into(), false));
    }

    #[test]
    fn a_long_configuration_name_is_cut_in_the_middle() {
        let name = "test_application_settings_and_more";
        let m = mono(name, Cut::Middle);
        // 20 characters: the end keeps 8, the beginning the rest.
        assert_eq!(
            m.fit(21.0 * CHAR),
            ("test_applica\u{2026}and_more".into(), true)
        );
        // 12 characters: the beginning keeps 8, the end 4.
        assert_eq!(m.fit(13.0 * CHAR), ("test_app\u{2026}more".into(), true));
        // Beyond `max_chars` the name is cut even with room to spare.
        let long = "a".repeat(30) + "_settings_tail";
        let m = mono(&long, Cut::Middle).max_chars(41);
        assert_eq!(m.width(), m.fit(f32::MAX).0.chars().count() as f32 * CHAR);
        let cut = format!("{}_s\u{2026}ngs_tail", "a".repeat(30));
        assert_eq!(m.fit(1000.0), (cut, true));
    }

    #[test]
    fn everything_keeps_its_width_when_it_fits() {
        let items = [
            Extent::fixed(30.0),
            Extent {
                min: 40.0,
                pref: 100.0,
            },
        ];
        assert_eq!(distribute(200.0, &items), [30.0, 100.0]);
    }

    #[test]
    fn the_widest_widget_gives_way_first() {
        let path = Extent {
            min: 60.0,
            pref: 400.0,
        };
        let project = Extent {
            min: 70.0,
            pref: 120.0,
        };
        let run = Extent {
            min: 50.0,
            pref: 110.0,
        };
        let fixed = Extent::fixed(100.0);
        // 100 too few: only the path shrinks.
        let w = distribute(
            100.0 + 400.0 + 120.0 + 110.0 - 100.0,
            &[fixed, path, project, run],
        );
        assert_eq!(w, [100.0, 300.0, 120.0, 110.0]);
        // The path down to the project's width: both shrink from there.
        let w = distribute(100.0 + 115.0 + 115.0 + 110.0, &[fixed, path, project, run]);
        assert_eq!(w, [100.0, 115.0, 115.0, 110.0]);
        // Too little for all minimums: every widget at its minimum.
        let w = distribute(100.0, &[fixed, path, project, run]);
        assert_eq!(w, [100.0, 60.0, 70.0, 50.0]);
    }

    #[test]
    fn the_shared_widths_fill_the_room_exactly() {
        let items = [
            Extent {
                min: 64.0,
                pref: 300.0,
            },
            Extent {
                min: 90.0,
                pref: 95.0,
            },
            Extent {
                min: 80.0,
                pref: 200.0,
            },
            Extent::fixed(237.0),
        ];
        for room in (500..900).step_by(7) {
            let room = room as f32;
            let w = distribute(room, &items);
            let sum: f32 = w.iter().sum();
            let expected = room.clamp(237.0 + 64.0 + 90.0 + 80.0, 237.0 + 300.0 + 95.0 + 200.0);
            assert!((sum - expected).abs() < 0.01, "{room}: {w:?}");
            for (w, e) in w.iter().zip(&items) {
                assert!(*w >= e.min - 0.001 && *w <= e.pref + 0.001);
            }
        }
    }

    fn overlap(p: &Placement) -> bool {
        let mut spans: Vec<(f32, f32)> = (0..3)
            .filter(|&g| !p.widths[g].is_empty())
            .map(|g| (p.x[g], p.x[g] + p.width(g)))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        spans.windows(2).any(|s| s[0].1 + 2.0 * GAP > s[1].0 + 0.01)
    }

    #[test]
    fn groups_never_overlap_and_the_right_one_slides_out() {
        let left = [Extent {
            min: 90.0,
            pref: 150.0,
        }];
        let centre = [Extent {
            min: 100.0,
            pref: 600.0,
        }];
        let right = [
            Extent {
                min: 80.0,
                pref: 120.0,
            },
            Extent::fixed(240.0),
        ];
        let mut last_right = f32::MAX;
        for width in (100..2000).rev().step_by(3) {
            let width = width as f32;
            let p = place(width, [&left, &centre, &right]);
            assert!(!overlap(&p), "{width}: {p:?}");
            assert_eq!(p.x[0], 0.0);
            // At its minimum the toolbar keeps its layout and the right
            // group runs past the edge.
            let min_total = 90.0 + 100.0 + 80.0 + 240.0 + 4.0 * GAP;
            if width < min_total {
                assert_eq!(p.x[2], 90.0 + 100.0 + 4.0 * GAP, "{width}");
                assert!(p.x[2] + p.width(2) > width);
            } else {
                assert!(p.x[2] + p.width(2) <= width + 0.01, "{width}: {p:?}");
            }
            assert!(p.x[2] <= last_right + 0.01);
            last_right = p.x[2];
        }
    }

    #[test]
    fn the_centre_group_is_centred_while_it_fits() {
        let left = [Extent::fixed(100.0)];
        let centre = [Extent::fixed(200.0)];
        let right = [Extent::fixed(300.0)];
        let p = place(1200.0, [&left, &centre, &right]);
        assert_eq!(p.x, [0.0, 500.0, 900.0]);
        // Less room on the right: it keeps two gaps from the right group.
        let p = place(800.0, [&left, &centre, &right]);
        assert_eq!(p.x, [0.0, 280.0, 500.0]);
        // No centre group: the right one stays at the right edge.
        let p = place(800.0, [&left, &[], &right]);
        assert_eq!(p.x[2], 500.0);
    }
}
