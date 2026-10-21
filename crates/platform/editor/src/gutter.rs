//! Laying out and painting the decorations ([`crate::decorations`]) in a
//! frame, and their geometry for hit testing, as the reference's gutter
//! (`EditorGutterComponentImpl`, `LineStatusMarkerDrawUtil`) and inline
//! debug renderer (`InlineDebugRendererBase`) draw them.

use gpui::{
    App, Bounds, Corners, Font, FontStyle, FontWeight, Hitbox, HitboxBehavior, Hsla, Pixels,
    ShapedLine, SharedString, TextRun, Window, fill, point, px, size,
};
use themes::{TextRole, UiRole};
use ui::icons::IconName;
use ui::theme::{Theme, color};

use crate::decorations::DecorationId;
use crate::editor::Editor;
use crate::element::LineLayout;

/// Size of a gutter icon (the reference's breakpoint icons).
pub(crate) const ICON_SIZE: f32 = 14.0;
/// Opacity of the owner's hover icon (`Breakpoint.iconHoverAlpha`).
const HOVER_ICON_ALPHA: f32 = 0.5;
/// The line numbers' area starts this far right of the gutter's left edge
/// (after the empty annotation area) and ends this far right of the
/// numbers (`gapAfterLineNumbersWidth`).
const LINE_NUMBERS_LEFT: f32 = 4.0;
const GAP_AFTER_LINE_NUMBERS: f32 = 4.0;
/// The change markers' lane: the reference's last gutter area (8 px, then
/// 1 px) before the text; a marker is 4 px wide, 3 px into the lane (1 px
/// for brace highlighters and 2 px more), and 3 px wider while hovered.
const MARKER_LANE: f32 = 8.0;
const MARKER_LANE_GAP: f32 = 1.0;
const MARKER_OFFSET: f32 = 3.0;
const MARKER_WIDTH: f32 = 4.0;
const MARKER_HOVER_EXTRA: f32 = 3.0;
/// Half the height of a deleted lines' marker (`getTriangleHeight`).
const DELETED_HALF: f32 = 5.0;
/// Full-line backgrounds start this far left of the gutter's right edge
/// (`getWhitespaceSeparatorOffset() + 1`).
pub(crate) const BACKGROUND_INTO_GUTTER: f32 = 2.0;
/// Room after an after-line text for the reference's drop-down triangle.
const AFTER_TEXT_ICON: f32 = 16.0;
/// How much of the editor's background a faded background is
/// (`TextDiffTypeFactory.MIDDLE_COLOR_FACTOR`).
const FADED_BACKGROUND: f32 = 0.6;
/// The gutter's whitespace separator is this far from the text
/// (`EditorGutterComponentImpl.getWhitespaceSeparatorOffset`, New UI); the
/// separator line is drawn one pixel nearer the text on a screen scaled
/// above 1, as the reference's Retina screens are.
const WHITESPACE_SEPARATOR: f32 = 3.0;
const SEPARATOR_LINE: f32 = 2.0;

/// `a` mixed with `b`, `factor` of `b` (the reference's `ColorUtil.mix`).
fn mix(a: Hsla, b: Hsla, factor: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    let m = |x: f32, y: f32| x * (1.0 - factor) + y * factor;
    gpui::Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    }
    .into()
}

/// The gutter's lanes in window x, for hit testing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Lanes {
    /// The line numbers' area (gutter icons, the owner's clicks).
    pub numbers: (Pixels, Pixels),
    /// The change markers' area.
    pub markers: (Pixels, Pixels),
}

impl Lanes {
    pub fn in_numbers(&self, x: Pixels) -> bool {
        x >= self.numbers.0 && x < self.numbers.1
    }

    pub fn in_markers(&self, x: Pixels) -> bool {
        x > self.markers.0 && x <= self.markers.1
    }
}

/// Where a change marker reacts to the mouse (window y; a deleted lines'
/// marker a third of a line above and below its boundary).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChangeHit {
    pub id: DecorationId,
    pub top: Pixels,
    pub bottom: Pixels,
}

/// A clickable after-line text in the last frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AfterHit {
    pub id: DecorationId,
    pub bounds: Bounds<Pixels>,
}

/// Decoration geometry of the last frame, kept for hit testing.
#[derive(Clone, Debug, Default)]
pub(crate) struct DecorationGeometry {
    pub lanes: Lanes,
    pub changes: Vec<ChangeHit>,
    pub after_texts: Vec<AfterHit>,
    /// The diff gutter's action icons.
    pub actions: Vec<AfterHit>,
}

impl DecorationGeometry {
    pub fn action_at(&self, position: gpui::Point<Pixels>) -> Option<DecorationId> {
        self.actions
            .iter()
            .find(|a| a.bounds.contains(&position))
            .map(|a| a.id)
    }

    pub fn change_at(&self, position: gpui::Point<Pixels>) -> Option<DecorationId> {
        if !self.lanes.in_markers(position.x) {
            return None;
        }
        self.changes
            .iter()
            .rev()
            .find(|c| position.y >= c.top && position.y < c.bottom)
            .map(|c| c.id)
    }

    pub fn after_text_at(&self, position: gpui::Point<Pixels>) -> Option<DecorationId> {
        self.after_texts
            .iter()
            .find(|a| a.bounds.contains(&position))
            .map(|a| a.id)
    }
}

/// An after-line text as painted.
pub(crate) struct AfterText {
    pub shaped: ShapedLine,
    pub origin: gpui::Point<Pixels>,
}

/// What prepaint computed for painting the decorations.
#[derive(Default)]
pub(crate) struct DecorationPaint {
    /// Lines whose number an icon replaces.
    pub hidden_numbers: Vec<usize>,
    /// Icons with their bounds and opacity (badges too).
    pub icons: Vec<(IconName, Bounds<Pixels>, f32)>,
    /// The line-number cells with an icon (hand pointer when the owner
    /// takes clicks there).
    pub icon_cells: Vec<Bounds<Pixels>>,
    /// The diff gutter's separator line before the text
    /// (`EditorGutterComponentImpl.paint`, `INDENT_GUIDE_COLOR`).
    pub separator: Option<(Bounds<Pixels>, Hsla)>,
    /// Full-line backgrounds, the lowest layer first.
    pub backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    /// Change markers: rounded bars, the hovered ones last.
    pub changes: Vec<(Bounds<Pixels>, Hsla)>,
    pub after_texts: Vec<AfterText>,
    /// Hitboxes of the hovered clickable texts and the change markers
    /// (hand pointer).
    pub hand_hitboxes: Vec<Hitbox>,
}

/// What of a frame the decorations are laid out in.
pub(crate) struct Frame<'a> {
    pub bounds: Bounds<Pixels>,
    /// Window x of text column 0 without scrolling (the gutter's right
    /// edge in the reference's terms).
    pub text_x: Pixels,
    /// The gutter's left edge and width.
    pub gutter_x: Pixels,
    pub gutter_width: Pixels,
    /// The gutter is at the right of the text ([`Editor::set_mirrored`]).
    pub mirrored: bool,
    pub scroll: gpui::Point<Pixels>,
    pub line_height: Pixels,
    pub char_width: Pixels,
    /// Offset of the baseline from a row's top.
    pub baseline: Pixels,
    pub font: &'a Font,
    pub font_size: Pixels,
    /// Window x of the right edge of the line numbers.
    pub numbers_right: Pixels,
    /// The rows drawn.
    pub lines: &'a [LineLayout],
    /// Rows of the text.
    pub total_rows: usize,
    /// No gutter (a field or a console): nothing drawn there.
    pub gutter: bool,
    /// The diff viewer's gutter ([`Editor::set_diff_gutter`]): a separator
    /// line before the text.
    pub diff_gutter: bool,
    /// The diff gutter has the column of the changes' actions.
    pub diff_actions: bool,
}

impl Frame<'_> {
    fn top_of(&self, row: usize) -> Pixels {
        self.bounds.top() + self.line_height * row as f32 - self.scroll.y
    }

    fn rows_of(&self, line: usize) -> impl Iterator<Item = &LineLayout> {
        let from = self.lines.partition_point(|l| l.line < line);
        self.lines[from..]
            .iter()
            .take_while(move |l| l.line == line)
    }

    pub fn lanes(&self) -> Lanes {
        if !self.gutter {
            return Lanes::default();
        }
        if self.mirrored {
            // The markers' lane next to the text, the line numbers at the
            // right edge.
            let markers_left = self.gutter_x + px(MARKER_LANE_GAP);
            return Lanes {
                numbers: (
                    markers_left + px(MARKER_LANE),
                    self.gutter_x + self.gutter_width,
                ),
                markers: (markers_left, markers_left + px(MARKER_LANE)),
            };
        }
        let markers_left = self.text_x - px(MARKER_LANE + MARKER_LANE_GAP);
        Lanes {
            numbers: (
                self.bounds.left() + px(LINE_NUMBERS_LEFT),
                self.numbers_right + px(GAP_AFTER_LINE_NUMBERS),
            ),
            markers: (markers_left, markers_left + px(MARKER_LANE)),
        }
    }

    /// The gutter's whitespace separator: where a diff change's gutter part
    /// ends and its text part begins.
    fn outline_x(&self) -> Pixels {
        if self.mirrored {
            self.gutter_x + px(WHITESPACE_SEPARATOR)
        } else {
            self.gutter_x + self.gutter_width - px(WHITESPACE_SEPARATOR)
        }
    }

    /// The gutter part (`true`) or the text part of a background drawn in
    /// two parts ([`crate::GutterFill`]).
    fn split_x(&self, gutter: bool) -> (Pixels, Pixels) {
        let outline = self.outline_x();
        match (gutter, self.mirrored) {
            (true, false) | (false, true) => (self.bounds.left(), outline),
            (false, false) | (true, true) => (outline, self.bounds.right()),
        }
    }

    /// Where a full-line background goes across: the text and a little of
    /// the gutter, or the whole width.
    fn background_x(&self, over_gutter: bool) -> (Pixels, Pixels) {
        match (over_gutter, self.mirrored) {
            (true, _) => (self.bounds.left(), self.bounds.right()),
            (false, false) => (
                self.text_x - px(BACKGROUND_INTO_GUTTER),
                self.bounds.right(),
            ),
            (false, true) => (
                self.bounds.left(),
                self.gutter_x + px(BACKGROUND_INTO_GUTTER),
            ),
        }
    }
}

/// Lays out the decorations of the visible lines.
pub(crate) fn prepaint(
    editor: &Editor,
    frame: &Frame<'_>,
    theme: &Theme,
    window: &mut Window,
) -> (DecorationPaint, DecorationGeometry) {
    let lanes = frame.lanes();
    let mut paint = DecorationPaint::default();
    if frame.gutter && frame.diff_gutter {
        let x = if frame.mirrored {
            frame.gutter_x + px(SEPARATOR_LINE)
        } else {
            frame.gutter_x + frame.gutter_width - px(SEPARATOR_LINE + 1.0)
        };
        paint.separator = Some((
            Bounds::new(
                point(x, frame.bounds.top()),
                size(px(1.0), frame.bounds.size.height),
            ),
            theme.color(UiRole::IndentGuide),
        ));
    }
    let mut geometry = DecorationGeometry {
        lanes,
        ..Default::default()
    };
    let decorations = &editor.decorations;
    let (Some(first), Some(last)) = (frame.lines.first(), frame.lines.last()) else {
        return (paint, geometry);
    };
    let visible = first.line..last.line + 1;
    let len_lines = editor.buffer.len_lines();
    let lh = frame.line_height;
    let all_lines = decorations.lines(&editor.buffer);
    let mut backgrounds: Vec<(i32, Bounds<Pixels>, Hsla)> = Vec::new();
    let mut changes: Vec<(bool, Bounds<Pixels>, Hsla)> = Vec::new();
    // After-line texts by line: the next text's left edge.
    let mut after_x: Vec<(usize, Pixels)> = Vec::new();
    let icon_size = px(ICON_SIZE);
    let icon_bounds = |row: &LineLayout| {
        let y = (lh - icon_size) / 2.0;
        let y = y.max(frame.baseline - icon_size).floor();
        Bounds::new(
            point(frame.numbers_right - icon_size, frame.top_of(row.row) + y),
            size(icon_size, icon_size),
        )
    };
    let cell = |row: &LineLayout| {
        Bounds::from_corners(
            point(lanes.numbers.0, frame.top_of(row.row)),
            point(lanes.numbers.1, frame.top_of(row.row) + lh),
        )
    };
    let badge_bounds = |icon: Bounds<Pixels>| {
        Bounds::new(
            point(
                icon.left() + px(crate::decorations::BADGE_OFFSET.0),
                icon.top() + px(crate::decorations::BADGE_OFFSET.1),
            ),
            size(
                px(crate::decorations::BADGE_SIZE),
                px(crate::decorations::BADGE_SIZE),
            ),
        )
    };
    // The action column of a diff gutter (`DiffGutterOperation`'s icons).
    let action_x = crate::side_by_side::diff_action_x(frame.text_x, frame.gutter_x, frame.mirrored);
    let action_size = px(crate::side_by_side::ACTION_SIZE);
    for (entry, lines) in decorations.entries().iter().zip(all_lines.iter()) {
        let d = &entry.decoration;
        let last_line = lines.end.max(lines.start + 1) - 1;
        let shown = lines.start < visible.end && last_line >= visible.start;
        // On its first line, or the line below the boundary it stands on.
        let action_line = lines.start.min(len_lines - 1);
        if frame.gutter
            && frame.diff_actions
            && let Some(action) = &d.action
            && visible.contains(&action_line)
            && let Some(row) = frame.rows_of(action_line).find(|l| l.sub == 0)
        {
            let y = ((lh - action_size) / 2.0).floor();
            let bounds = Bounds::new(
                point(action_x, frame.top_of(row.row) + y),
                size(action_size, action_size),
            );
            paint.icons.push((action.icon, bounds, 1.0));
            geometry.actions.push(AfterHit {
                id: entry.id,
                bounds,
            });
        }
        if frame.gutter
            && let Some(icon) = &d.gutter_icon
            && visible.contains(&lines.start)
            && !paint.hidden_numbers.contains(&lines.start)
            && let Some(row) = frame.rows_of(lines.start).find(|l| l.sub == 0)
        {
            paint.hidden_numbers.push(lines.start);
            let bounds = icon_bounds(row);
            paint.icons.push((icon.icon, bounds, 1.0));
            if let Some(badge) = icon.badge {
                paint.icons.push((badge, badge_bounds(bounds), 1.0));
            }
            paint.icon_cells.push(cell(row));
        }
        if shown && let Some(bg) = d.background {
            let full = theme.color(bg.color);
            let faded = mix(
                full,
                theme.color(UiRole::EditorBackground),
                FADED_BACKGROUND,
            );
            let c = if bg.faded { faded } else { full };
            // A diff change's gutter part in its own paint mode
            // (`DiffLineMarkerRenderer`): up to the whitespace separator,
            // the text's colour after it.
            let fill = bg.gutter.filter(|_| frame.gutter);
            let parts = match fill {
                Some(f) => [
                    (frame.split_x(false), c),
                    (frame.split_x(true), if f.faded { faded } else { full }),
                ],
                None => [(frame.background_x(bg.over_gutter), c); 2],
            };
            let parts = if fill.is_some() {
                &parts[..]
            } else {
                &parts[..1]
            };
            for line in lines.start.max(visible.start)..lines.end.min(visible.end) {
                for row in frame.rows_of(line) {
                    for &((x0, x1), c) in parts {
                        backgrounds.push((
                            bg.layer,
                            Bounds::from_corners(
                                point(x0, frame.top_of(row.row)),
                                point(x1, frame.top_of(row.row) + lh),
                            ),
                            c,
                        ));
                    }
                }
            }
            // A change left out of the commit: a line of the full colour
            // along the top and the bottom of its gutter part
            // (`EXCLUDED_GUTTER`, `drawChunkBorderLine`).
            if fill.is_some_and(|f| f.border) && !lines.is_empty() {
                let (x0, x1) = frame.split_x(true);
                let row_top = |line: usize| {
                    if line >= len_lines {
                        frame.top_of(frame.total_rows)
                    } else {
                        frame.top_of(editor.line_row(line))
                    }
                };
                for y in [row_top(lines.start), row_top(lines.end) - px(1.0)] {
                    backgrounds.push((
                        bg.layer,
                        Bounds::from_corners(point(x0, y), point(x1, y + px(1.0))),
                        full,
                    ));
                }
            }
        }
        // A background between lines: a line on the boundary, two pixels
        // thick (inside the view at the text's top).
        if entry.between
            && let Some(bg) = d.background
        {
            let y = if lines.start >= len_lines {
                frame.top_of(frame.total_rows)
            } else {
                frame.top_of(editor.line_row(lines.start))
            };
            let y = if lines.start == 0 { y + px(1.0) } else { y };
            if y >= frame.bounds.top() - px(2.0) && y <= frame.bounds.bottom() + px(2.0) {
                let (x0, x1) = frame.background_x(bg.over_gutter);
                backgrounds.push((
                    bg.layer,
                    Bounds::from_corners(point(x0, y - px(1.0)), point(x1, y + px(1.0))),
                    theme.color(bg.color),
                ));
            }
        }
        if frame.gutter
            && let Some(kind) = d.change
        {
            let row_top = |line: usize| {
                if line >= len_lines {
                    frame.top_of(frame.total_rows)
                } else {
                    frame.top_of(editor.line_row(line))
                }
            };
            let y1 = row_top(lines.start);
            let y2 = if lines.is_empty() {
                y1
            } else {
                row_top(lines.end)
            };
            let view = frame.bounds.top() - lh..frame.bounds.bottom() + lh;
            if y2 >= view.start && y1 <= view.end {
                let hovered = editor.hovered_change == Some(entry.id);
                let x1 = lanes.markers.0 + px(MARKER_OFFSET);
                let x0 = if hovered {
                    x1 - px(MARKER_HOVER_EXTRA)
                } else {
                    x1
                };
                let x2 = x1 + px(MARKER_WIDTH);
                let c = theme.color(kind.color());
                if y1 == y2 {
                    // Deleted lines: a short bar on the boundary.
                    let top_limit = frame.bounds.top() - frame.scroll.y + px(DELETED_HALF);
                    let y = y1.max(top_limit);
                    let half = px(DELETED_HALF);
                    changes.push((
                        hovered,
                        Bounds::from_corners(
                            point(x0, y - half + px(1.0)),
                            point(x2, y + half - px(1.0)),
                        ),
                        c,
                    ));
                    let gap = lh / 3.0;
                    geometry.changes.push(ChangeHit {
                        id: entry.id,
                        top: y1 - gap,
                        bottom: y1 + gap,
                    });
                } else {
                    changes.push((
                        hovered,
                        Bounds::from_corners(point(x0, y1 + px(1.0)), point(x2, y2 - px(1.0))),
                        c,
                    ));
                    geometry.changes.push(ChangeHit {
                        id: entry.id,
                        top: y1,
                        bottom: y2,
                    });
                }
            }
        }
        if let Some(after) = &d.after_text
            && visible.contains(&last_line)
            && let Some(row) = frame.rows_of(last_line).find(|l| l.last)
        {
            let left = match after_x.iter().find(|(l, _)| *l == last_line) {
                Some((_, x)) => *x,
                None => {
                    let end = editor.buffer.line_len(last_line);
                    frame.text_x - frame.scroll.x + row.x_for_col(end) + frame.char_width
                }
            };
            let hovered = after.clickable && editor.hovered_after == Some(entry.id);
            let link = theme
                .text_style(TextRole::Link)
                .fg
                .map(color)
                .unwrap_or(theme.color(UiRole::EditorText));
            let mut text = String::new();
            let mut runs = Vec::with_capacity(after.pieces.len());
            for piece in &after.pieces {
                let style = theme.text_style(piece.role);
                let mut font = frame.font.clone();
                if style.is_bold() {
                    font.weight = FontWeight::BOLD;
                }
                if style.is_italic() || piece.italic {
                    font.style = FontStyle::Italic;
                }
                let fg = style
                    .fg
                    .map(color)
                    .unwrap_or(theme.color(UiRole::EditorText));
                text.push_str(&piece.text);
                runs.push(TextRun {
                    len: piece.text.len(),
                    font,
                    color: if hovered { link } else { fg },
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
            let shaped = window.text_system().shape_line(
                SharedString::from(text),
                frame.font_size,
                &runs,
                None,
            );
            // The reference's inlay: its text half a space in, then room
            // for two spaces and the drop-down triangle.
            let x = left + frame.char_width * 1.5;
            let top = frame.top_of(row.row);
            let width = shaped.width;
            if after.clickable {
                geometry.after_texts.push(AfterHit {
                    id: entry.id,
                    bounds: Bounds::new(point(x, top), size(width, lh)),
                });
            }
            let next = left + width + frame.char_width * 2.0 + px(AFTER_TEXT_ICON);
            match after_x.iter_mut().find(|(l, _)| *l == last_line) {
                Some(slot) => slot.1 = next,
                None => after_x.push((last_line, next)),
            }
            paint.after_texts.push(AfterText {
                shaped,
                origin: point(x, top),
            });
        }
    }
    // The owner's hover icon on a line without an icon.
    if frame.gutter
        && let (Some(line), Some((icon_line, icon))) =
            (editor.gutter_hover, editor.gutter_hover_icon)
        && line == icon_line
        && !paint.hidden_numbers.contains(&line)
        && let Some(row) = frame.rows_of(line).find(|l| l.sub == 0)
    {
        paint.hidden_numbers.push(line);
        paint.icons.push((icon, icon_bounds(row), HOVER_ICON_ALPHA));
        paint.icon_cells.push(cell(row));
    }
    paint.backgrounds = in_layer_order(backgrounds);
    changes.sort_by_key(|(hovered, _, _)| *hovered);
    paint.changes = changes.into_iter().map(|(_, b, c)| (b, c)).collect();
    (paint, geometry)
}

/// Full-line backgrounds in paint order: the lowest layer first, ties in
/// the order they were added (the execution point over a breakpoint).
fn in_layer_order<T>(mut backgrounds: Vec<(i32, Bounds<Pixels>, T)>) -> Vec<(Bounds<Pixels>, T)> {
    backgrounds.sort_by_key(|(layer, _, _)| *layer);
    backgrounds.into_iter().map(|(_, b, c)| (b, c)).collect()
}

/// Hitboxes with the hand pointer: the change markers, the clickable
/// after-line texts and, when the owner takes clicks on the line numbers,
/// the line-number cells with an icon (the reference's gutter gives icons
/// with a click action the hand).
pub(crate) fn insert_hitboxes(
    paint: &mut DecorationPaint,
    geometry: &DecorationGeometry,
    bounds: Bounds<Pixels>,
    line_number_clicks: bool,
    window: &mut Window,
) {
    let lanes = geometry.lanes;
    if line_number_clicks {
        for cell in std::mem::take(&mut paint.icon_cells) {
            let rect = cell.intersect(&bounds);
            if rect.size.height > px(0.0) {
                paint
                    .hand_hitboxes
                    .push(window.insert_hitbox(rect, HitboxBehavior::Normal));
            }
        }
    }
    for c in &geometry.changes {
        let rect = Bounds::from_corners(
            point(lanes.markers.0, c.top),
            point(lanes.markers.1, c.bottom),
        )
        .intersect(&bounds);
        if rect.size.height > px(0.0) {
            paint
                .hand_hitboxes
                .push(window.insert_hitbox(rect, HitboxBehavior::Normal));
        }
    }
    for a in geometry.after_texts.iter().chain(&geometry.actions) {
        let rect = a.bounds.intersect(&bounds);
        if rect.size.width > px(0.0) {
            paint
                .hand_hitboxes
                .push(window.insert_hitbox(rect, HitboxBehavior::Normal));
        }
    }
}

/// Paints the diff gutter's separator line (under the backgrounds: a
/// change's background covers it).
pub(crate) fn paint_separator(paint: &DecorationPaint, window: &mut Window) {
    if let Some((b, c)) = paint.separator {
        window.paint_quad(fill(b, c));
    }
}

/// Paints the full-line backgrounds (over the caret row, under the
/// selection).
pub(crate) fn paint_backgrounds(paint: &DecorationPaint, window: &mut Window) {
    for (b, c) in &paint.backgrounds {
        window.paint_quad(fill(*b, *c));
    }
}

/// Paints the gutter's icons and change markers.
pub(crate) fn paint_gutter(paint: &DecorationPaint, window: &mut Window, cx: &App) {
    for (b, c) in &paint.changes {
        window.paint_quad(fill(*b, *c).corner_radii(b.size.width / 2.0));
    }
    for (icon, b, alpha) in &paint.icons {
        if let Some(image) =
            ui::icons::image_with_opacity(*icon, b.size.width, None, *alpha, window, cx)
        {
            let _ = window.paint_image(*b, Corners::default(), image, 0, false);
        }
    }
}

/// Paints the texts after line ends.
pub(crate) fn paint_after_texts(
    paint: &DecorationPaint,
    line_height: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    for t in &paint.after_texts {
        let _ = t.shaped.paint(t.origin, line_height, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decorations::layer;

    #[test]
    fn the_execution_point_is_drawn_over_a_breakpoint_over_the_caret_row() {
        let b = Bounds::default();
        let order = in_layer_order(vec![
            (layer::EXECUTION_POINT, b, "execution point"),
            (layer::BREAKPOINT, b, "breakpoint"),
            (layer::BREAKPOINT, b, "second breakpoint"),
        ]);
        let names: Vec<&str> = order.into_iter().map(|(_, n)| n).collect();
        assert_eq!(
            names,
            vec!["breakpoint", "second breakpoint", "execution point"]
        );
        // The caret row is painted before every decoration background.
        const { assert!(layer::CARET_ROW < layer::BREAKPOINT) };
    }
}
