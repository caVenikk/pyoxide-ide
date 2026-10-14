//! The editor's custom element: lays out and paints only the visible lines
//! (virtualized), the gutter with line numbers, the current line, selections,
//! the caret and a scrollbar, and routes mouse input to the editor.

use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle, Style,
    TextRun, UnderlineStyle, Window, fill, outline, point, px, relative, size,
};
use text::movement;
use themes::{Metric, TextRole, UiRole, UnderlineStyle as ThemeUnderline};
use ui::theme::{Theme, ThemeExt, color};

use crate::diagnostics::{Analysis, Look, MarkSeverity, line_looks};
use crate::editor::{DragUnit, Editor, EditorEvent};
use crate::highlight::InlineStyle;

/// Lines longer than this are shaped only around the visible columns.
pub const LONG_LINE: usize = 2000;
/// Extra columns shaped on each side of the view on long lines.
const LONG_LINE_MARGIN: usize = 200;
/// Space left of the line numbers.
const GUTTER_LEFT: f32 = 8.0;
/// Space right of the line numbers (room for breakpoints, change markers
/// and folding).
const GUTTER_RIGHT: f32 = 42.5;
/// Space between the gutter and column 0 of the text.
const TEXT_LEFT: f32 = 6.0;
/// Room left of a console's text (the reference shows no gutter there;
/// its folding outline is empty).
const CONSOLE_GUTTER: f32 = 6.0;
/// Smallest scrollbar thumb.
const MIN_THUMB: f32 = 24.0;
/// Height of a search match mark on the scrollbar.
const STRIPE_MARK: f32 = 2.0;
/// Size of the dots of a dotted underline, and of the gaps between them
/// (the reference's bold dotted line: dots as large as the line is thick).
const DOT: f32 = 2.0;

/// One painted line: its shaped text and the mapping from char columns to
/// bytes of the shaped text (tabs are expanded to spaces).
#[derive(Clone)]
pub(crate) struct LineLayout {
    pub line: usize,
    /// First char column that was shaped (non-zero only on long lines).
    pub start_col: usize,
    /// X of `start_col` relative to column 0.
    pub x_offset: Pixels,
    pub shaped: ShapedLine,
    /// Byte offset in the shaped text of each char column from `start_col`
    /// (one more entry than chars); `None` when columns are bytes.
    pub col_bytes: Option<Vec<u32>>,
}

impl LineLayout {
    /// X of a char column relative to column 0 of the text.
    pub fn x_for_col(&self, col: usize) -> Pixels {
        let c = col.saturating_sub(self.start_col);
        let byte = match &self.col_bytes {
            Some(v) => v[c.min(v.len() - 1)] as usize,
            None => c.min(self.shaped.len()),
        };
        self.x_offset + self.shaped.x_for_index(byte)
    }

    /// The char column nearest to `x` (relative to column 0).
    pub fn col_for_x(&self, x: Pixels) -> usize {
        let byte = self.shaped.closest_index_for_x(x - self.x_offset);
        let c = match &self.col_bytes {
            Some(v) => v.partition_point(|b| (*b as usize) < byte),
            None => byte,
        };
        self.start_col + c
    }
}

/// Geometry of the last frame, kept by the editor for hit testing, paging
/// and the input method.
#[derive(Clone)]
pub(crate) struct LayoutInfo {
    pub bounds: Bounds<Pixels>,
    /// Window x of text column 0 without horizontal scrolling.
    pub text_x: Pixels,
    pub line_height: Pixels,
    pub char_width: Pixels,
    pub visible: Range<usize>,
    pub lines: Vec<LineLayout>,
    pub viewport_lines: usize,
    pub scrollbar: Option<Scrollbar>,
    /// The error stripe at the right edge (the scrollbar's track when the
    /// text scrolls): marks of problems and search matches.
    pub stripe: Option<Bounds<Pixels>>,
    /// Problem marks on the error stripe, in paint order (the most severe
    /// last).
    pub problem_stripes: Vec<StripeMark>,
    /// Hyperlinks on the visible lines.
    pub links: Vec<LinkRect>,
}

/// A problem's mark on the error stripe.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StripeMark {
    pub bounds: Bounds<Pixels>,
    pub color: Hsla,
    /// Index of the problem in the editor's marks.
    pub mark: usize,
}

/// How far from a stripe mark the mouse still hovers or clicks it (the
/// reference's twice the smallest mark height on either side).
const STRIPE_SLOP: f32 = STRIPE_MARK * 2.0;

impl LayoutInfo {
    /// The problems whose stripe marks are near a window y, the nearest
    /// (by the top of its mark) first; each problem once.
    pub fn stripe_marks_near(&self, y: Pixels) -> Vec<usize> {
        let slop = px(STRIPE_SLOP);
        let mut near: Vec<(Pixels, usize)> = self
            .problem_stripes
            .iter()
            .filter(|s| y >= s.bounds.top() - slop && y <= s.bounds.bottom() + slop)
            .map(|s| ((s.bounds.top() - y).abs(), s.mark))
            .collect();
        near.sort_by(|a, b| {
            f32::from(a.0)
                .total_cmp(&f32::from(b.0))
                .then(a.1.cmp(&b.1))
        });
        let mut marks: Vec<usize> = Vec::with_capacity(near.len());
        for (_, mark) in near {
            if !marks.contains(&mark) {
                marks.push(mark);
            }
        }
        marks
    }

    /// The window y of the middle of a problem's stripe mark.
    pub fn stripe_mark_middle(&self, mark: usize) -> Option<Pixels> {
        self.problem_stripes
            .iter()
            .find(|s| s.mark == mark)
            .map(|s| s.bounds.center().y)
    }

    /// Whether a window position is over the text (not the gutter, the
    /// error stripe or outside).
    pub fn over_text(&self, position: gpui::Point<Pixels>) -> bool {
        self.bounds.contains(&position)
            && position.x >= self.text_x
            && self.stripe.is_none_or(|s| position.x < s.left())
    }
}

/// A hyperlink's part on one visible line.
#[derive(Clone)]
pub(crate) struct LinkRect {
    pub bounds: Bounds<Pixels>,
    pub line: usize,
    /// Char range in the line.
    pub range: Range<usize>,
}

impl LayoutInfo {
    /// The hyperlink at a window position.
    pub fn link_at(&self, position: gpui::Point<Pixels>) -> Option<&LinkRect> {
        self.links.iter().find(|l| l.bounds.contains(&position))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Scrollbar {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
}

impl LayoutInfo {
    pub fn line(&self, line: usize) -> Option<&LineLayout> {
        let i = line.checked_sub(self.visible.start)?;
        self.lines.get(i).filter(|l| l.line == line)
    }
}

impl Editor {
    /// Char offset under a window position (clamped into the text).
    pub(crate) fn offset_for_position(&self, position: gpui::Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else {
            return 0;
        };
        let y = position.y - layout.bounds.top() + self.scroll.y;
        let last = self.buffer.len_lines() - 1;
        let line = if y < px(0.0) {
            0
        } else {
            ((y / layout.line_height).floor() as usize).min(last)
        };
        let x = position.x - layout.text_x + self.scroll.x;
        let col = match layout.line(line) {
            Some(l) => l.col_for_x(x),
            None => (x / layout.char_width).round().max(0.0) as usize,
        };
        self.buffer.line_start(line) + col.min(self.buffer.line_len(line))
    }

    /// Window bounds of the char at `offset` (for the input method).
    pub(crate) fn bounds_for_offset(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let p = self.buffer.offset_to_point(offset);
        let x = match layout.line(p.line) {
            Some(l) => l.x_for_col(p.column),
            None => layout.char_width * p.column as f32,
        };
        let top = layout.bounds.top() + layout.line_height * p.line as f32 - self.scroll.y;
        Some(Bounds::new(
            point(layout.text_x + x - self.scroll.x, top),
            size(layout.char_width, layout.line_height),
        ))
    }

    /// The char under a window position in the last frame: `None` outside
    /// the text area, below the last line and past the end of a line,
    /// except over a problem drawn after the end of its line (then the
    /// line end).
    pub fn char_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        if !layout.over_text(position) {
            return None;
        }
        let y = position.y - layout.bounds.top() + self.scroll.y;
        let line = (y / layout.line_height).floor();
        if line < 0.0 || line as usize >= self.buffer.len_lines() {
            return None;
        }
        let line = line as usize;
        let x = position.x - layout.text_x + self.scroll.x;
        let len = self.buffer.line_len(line);
        let (col, end_x) = match layout.line(line) {
            Some(l) => {
                let col = l.col_for_x(x);
                // The char whose cell holds `x`, not the nearest boundary.
                let col = if col > 0 && l.x_for_col(col) > x {
                    col - 1
                } else {
                    col
                };
                (col, l.x_for_col(len))
            }
            None => (
                (x / layout.char_width).floor().max(0.0) as usize,
                layout.char_width * len as f32,
            ),
        };
        let line_start = self.buffer.line_start(line);
        if col < len {
            return Some(line_start + col);
        }
        // The cell after the line end shows the empty problems of the line.
        if x >= end_x + layout.char_width {
            return None;
        }
        self.diagnostics.empty_in(line_start..line_start + len)
    }
}

pub(crate) struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// What prepaint computed for paint.
pub(crate) struct Prepaint {
    hitbox: Hitbox,
    /// The text area without the gutter and the scrollbar (I-beam cursor).
    text_hitbox: Hitbox,
    layout: LayoutInfo,
    gutter_width: Pixels,
    numbers: Vec<(ShapedLine, gpui::Point<Pixels>)>,
    current_lines: Vec<Bounds<Pixels>>,
    selections: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
    search: Vec<SearchRect>,
    /// Marks of search matches on the scrollbar.
    stripe_marks: Vec<Bounds<Pixels>>,
    /// The problems' stripe marks as painted: one rectangle per run of
    /// pixel rows in one colour.
    problem_spans: Vec<(Bounds<Pixels>, Hsla)>,
    /// The placeholder of an empty one-line field and where it goes.
    placeholder: Option<(ShapedLine, gpui::Point<Pixels>)>,
    /// Hitboxes of the hyperlinks (hand pointer).
    link_hitboxes: Vec<Hitbox>,
    /// Hitboxes of the link highlight (hand pointer).
    link_highlight_hitboxes: Vec<Hitbox>,
    /// Dotted underlines of problems (the text runs draw only solid and
    /// wavy ones).
    dotted: Vec<Dotted>,
    /// Problems drawn after the end of a line.
    after_end: Vec<AfterEnd>,
    colors: Colors,
}

/// A dotted underline from `x0` to `x1` with its top at `y`.
#[derive(Clone, Copy, Debug)]
struct Dotted {
    x0: Pixels,
    x1: Pixels,
    y: Pixels,
    color: Hsla,
}

/// The decoration of a problem after the end of a line: one char cell
/// with the look of the problem, as the reference paints a highlighter
/// "after end of line".
#[derive(Clone, Copy, Debug)]
struct AfterEnd {
    cell: Bounds<Pixels>,
    /// Top of an underline in the cell.
    underline_y: Pixels,
    /// Top of a strikethrough in the cell.
    strike_y: Pixels,
    look: Look,
    /// The text colour, for effects without a colour of their own.
    text: Hsla,
}

/// One line's part of a search match.
#[derive(Clone, Copy)]
struct SearchRect {
    bounds: Bounds<Pixels>,
    current: bool,
    excluded: bool,
}

#[derive(Clone, Copy)]
struct Colors {
    background: Hsla,
    gutter: Hsla,
    current_line: Hsla,
    selection: Hsla,
    caret: Hsla,
    scrollbar: Hsla,
    scrollbar_hover: Hsla,
    search_match: Hsla,
    search_stripe: Hsla,
    search_current: Hsla,
    /// Background of the current match (search within a selection, where
    /// the current match is not selected).
    search_current_background: Option<Hsla>,
    text: Hsla,
}

/// Font metrics of the editor font at the editor size.
pub(crate) struct Metrics {
    font: Font,
    font_size: Pixels,
    pub(crate) line_height: Pixels,
    char_width: Pixels,
    /// Ascent and descent of the font (both positive), for decorations
    /// drawn like the text's own underlines.
    ascent: Pixels,
    descent: Pixels,
    /// Line numbers are one size smaller than the text.
    number_size: Pixels,
    number_width: Pixels,
}

/// OpenType features of the UI font in a text field: no ligatures and no
/// contextual alternates, so that typed characters show as typed (`->`
/// stays two characters instead of an arrow, as in the reference IDE).
pub(crate) fn field_font_features() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![
        ("calt".into(), 0),
        ("liga".into(), 0),
    ]))
}

/// Font metrics of an editor: the editor font, or the UI font of a one-line
/// field that shows UI text (search fields).
pub(crate) fn metrics(theme: &Theme, window: &Window, ui_font: bool) -> Metrics {
    let font = Font {
        family: if ui_font {
            theme.ui_family()
        } else {
            theme.mono_family()
        },
        features: if ui_font {
            field_font_features()
        } else if theme.raw().fonts.editor_ligatures {
            FontFeatures::default()
        } else {
            FontFeatures::disable_ligatures()
        },
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let font_size = if ui_font {
        theme.ui_size()
    } else {
        theme.editor_size()
    };
    let ts = window.text_system();
    let id = ts.resolve_font(&font);
    // Whole-pixel ascent and descent (the descent is reported negative),
    // times the line spacing, rounded up: 13 px JetBrains Mono at 1.2 gives
    // 22 px lines, 14 px gives 24.
    let ascent = f32::from(ts.ascent(id, font_size)).ceil();
    let descent = f32::from(ts.descent(id, font_size)).abs().ceil();
    let line_height = px(((ascent + descent) * theme.editor_line_spacing())
        .ceil()
        .max(1.0));
    let advance = |size: Pixels| {
        ts.advance(id, size, '0')
            .map(|s| s.width)
            .unwrap_or(size * 0.6)
    };
    let number_size = font_size - px(1.0);
    Metrics {
        font,
        font_size,
        line_height,
        char_width: advance(font_size),
        ascent: ts.ascent(id, font_size),
        descent: ts.descent(id, font_size).abs(),
        number_size,
        number_width: advance(number_size),
    }
}

impl Metrics {
    /// Offsets from the top of a line of the underline and the
    /// strikethrough of its text, where the text system draws them.
    fn effect_offsets(&self) -> (Pixels, Pixels) {
        let padding_top = (self.line_height - self.ascent - self.descent) / 2.0;
        let baseline = padding_top + self.ascent;
        (
            baseline + self.descent * 0.618,
            (self.ascent * 0.5 + baseline) * 0.5,
        )
    }
}

/// Display text of a line (tabs expanded), the first shaped column and the
/// column-to-byte map.
fn display_text(
    editor: &Editor,
    line: usize,
    cols: Range<usize>,
    tab_size: usize,
) -> (String, Option<Vec<u32>>) {
    let slice = editor.buffer.line(line);
    let mut text = String::with_capacity(cols.len() + 8);
    let mut map: Option<Vec<u32>> = None;
    let mut display_col = 0usize;
    for (i, c) in slice.chars_at(cols.start).take(cols.len()).enumerate() {
        if c == '\t' || !c.is_ascii() {
            map.get_or_insert_with(|| (0..=i as u32).collect::<Vec<_>>());
        }
        if c == '\t' {
            let n = tab_size - display_col % tab_size;
            text.extend(std::iter::repeat_n(' ', n));
            display_col += n;
        } else {
            text.push(c);
            display_col += 1;
        }
        if let Some(m) = &mut map {
            if m.len() == i + 1 {
                m.push(text.len() as u32);
            } else {
                m[i + 1] = text.len() as u32;
            }
        }
    }
    (text, map)
}

/// Splits `runs` so that the bytes `range` can be restyled by `f`.
fn map_range(runs: Vec<TextRun>, range: Range<usize>, f: impl Fn(&mut TextRun)) -> Vec<TextRun> {
    if range.start >= range.end {
        return runs;
    }
    let mut split = Vec::with_capacity(runs.len() + 2);
    let mut pos = 0;
    for run in runs {
        let (rs, re) = (pos, pos + run.len);
        pos = re;
        for (s, e, inside) in [
            (rs, range.start.min(re), false),
            (range.start.max(rs), range.end.min(re), true),
            (range.end.max(rs), re, false),
        ] {
            if s < e {
                let mut r = run.clone();
                r.len = e - s;
                if inside {
                    f(&mut r);
                }
                split.push(r);
            }
        }
    }
    split
}

/// Splits `runs` so that the bytes `range` get `underline`.
fn underline_range(
    runs: Vec<TextRun>,
    range: Range<usize>,
    underline: UnderlineStyle,
) -> Vec<TextRun> {
    map_range(runs, range, |r| {
        r.underline = Some(UnderlineStyle {
            color: underline.color.or(Some(r.color)),
            ..underline
        });
    })
}

/// Applies an explicit style to a run.
fn apply_inline(r: &mut TextRun, style: &InlineStyle) {
    if let Some(c) = style.color {
        r.color = c;
    }
    if style.background.is_some() {
        r.background_color = style.background;
    }
    if style.bold {
        r.font.weight = FontWeight::BOLD;
    }
    if style.italic {
        r.font.style = FontStyle::Italic;
    }
    if let Some(thickness) = style.underline {
        r.underline = Some(UnderlineStyle {
            color: Some(r.color),
            thickness: px(thickness),
            wavy: false,
        });
    }
    if style.strikethrough {
        r.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.0),
            color: Some(r.color),
        });
    }
}

/// Draws the look of the problems over a stretch of a run (dotted
/// underlines are painted separately).
fn apply_look(r: &mut TextRun, look: &Look) {
    if let Some(fg) = look.fg {
        r.color = fg;
    }
    if look.bg.is_some() {
        r.background_color = look.bg;
    }
    if look.bold {
        r.font.weight = FontWeight::BOLD;
    }
    if look.italic {
        r.font.style = FontStyle::Italic;
    }
    if let Some(u) = look.underline {
        r.underline = match u.kind {
            ThemeUnderline::None | ThemeUnderline::Dotted => None,
            kind => Some(underline_of(kind, u.color.unwrap_or(r.color))),
        };
    }
    if look.strikethrough {
        r.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.0),
            color: Some(r.color),
        });
    }
}

/// A solid or wavy underline of a theme kind (double and dashed lines are
/// drawn single).
fn underline_of(kind: ThemeUnderline, color: Hsla) -> UnderlineStyle {
    UnderlineStyle {
        color: Some(color),
        thickness: px(if kind == ThemeUnderline::Thick {
            2.0
        } else {
            1.0
        }),
        wavy: kind == ThemeUnderline::Wavy,
    }
}

/// The theme's link style as an explicit style.
fn link_style(theme: &Theme, default: Hsla) -> InlineStyle {
    let style = theme.text_style(TextRole::Link);
    let fg = style.fg.map(color).unwrap_or(default);
    InlineStyle {
        color: Some(fg),
        background: style.bg.map(color),
        bold: style.is_bold(),
        italic: style.is_italic(),
        underline: Some(1.0),
        strikethrough: false,
    }
}

/// The theme's underline for syntax errors.
fn error_underline(theme: &Theme) -> UnderlineStyle {
    let style = theme.text_style(TextRole::DiagnosticError);
    UnderlineStyle {
        color: style.underline_colour().map(color).or(style.fg.map(color)),
        thickness: px(1.0),
        wavy: !matches!(
            style.underline_style(),
            ThemeUnderline::Single | ThemeUnderline::Thick
        ),
    }
}

/// What decorates one line besides its role spans.
struct LineExtras<'a> {
    errors: &'a [Range<usize>],
    styles: &'a [(Range<usize>, InlineStyle)],
    links: &'a [Range<usize>],
    /// Problems (char columns of the line) and how they look.
    diagnostics: &'a [(Range<usize>, Look)],
    /// The link highlight's char columns on the line.
    link_highlight: Option<Range<usize>>,
}

/// Text runs for a line from highlight spans (char columns of the line),
/// explicit styles and links drawn over them, with syntax error marks
/// underlined, the problems drawn over all that and the input method's
/// composition underlined.
#[allow(clippy::too_many_arguments)]
fn runs(
    spans: &[(Range<usize>, TextRole)],
    extras: LineExtras<'_>,
    cols: &Range<usize>,
    col_bytes: Option<&Vec<u32>>,
    text_len: usize,
    base: &Font,
    default: Hsla,
    theme: &Theme,
    marked: Option<Range<usize>>,
) -> Vec<TextRun> {
    let byte_of = |col: usize| -> usize {
        let c = col.clamp(cols.start, cols.end) - cols.start;
        match col_bytes {
            Some(v) => v[c.min(v.len() - 1)] as usize,
            None => c.min(text_len),
        }
    };
    let plain = TextRun {
        len: 0,
        font: base.clone(),
        color: default,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut out: Vec<TextRun> = Vec::new();
    let mut at = 0usize;
    fn push(out: &mut Vec<TextRun>, at: &mut usize, end: usize, run: &TextRun) {
        if end > *at {
            out.push(TextRun {
                len: end - *at,
                ..run.clone()
            });
            *at = end;
        }
    }
    for (range, role) in spans {
        let start = byte_of(range.start);
        let end = byte_of(range.end);
        if end <= at || start >= text_len {
            continue;
        }
        let from = start.max(at);
        push(&mut out, &mut at, from, &plain);
        let style = theme.text_style(*role);
        let mut font = base.clone();
        if style.is_bold() {
            font.weight = FontWeight::BOLD;
        }
        if style.is_italic() {
            font.style = FontStyle::Italic;
        }
        let fg = style.fg.map(color).unwrap_or(default);
        let underline = match style.underline_style() {
            ThemeUnderline::None => None,
            u => Some(UnderlineStyle {
                color: Some(style.underline_colour().map(color).unwrap_or(fg)),
                thickness: px(if u == ThemeUnderline::Thick { 2.0 } else { 1.0 }),
                wavy: u == ThemeUnderline::Wavy,
            }),
        };
        let run = TextRun {
            len: 0,
            font,
            color: fg,
            background_color: style.bg.map(color),
            underline,
            strikethrough: style.is_strikethrough().then_some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(fg),
            }),
        };
        push(&mut out, &mut at, end.min(text_len), &run);
    }
    push(&mut out, &mut at, text_len, &plain);
    for (range, style) in extras.styles {
        out = map_range(out, byte_of(range.start)..byte_of(range.end), |r| {
            apply_inline(r, style)
        });
    }
    if !extras.links.is_empty() {
        let style = link_style(theme, default);
        for range in extras.links {
            out = map_range(out, byte_of(range.start)..byte_of(range.end), |r| {
                apply_inline(r, &style)
            });
        }
    }
    if !extras.errors.is_empty() {
        let underline = error_underline(theme);
        for e in extras.errors {
            out = underline_range(out, byte_of(e.start)..byte_of(e.end), underline);
        }
    }
    for (range, look) in extras.diagnostics {
        out = map_range(out, byte_of(range.start)..byte_of(range.end), |r| {
            apply_look(r, look)
        });
    }
    // The symbol under the mouse with the navigation modifier, over the
    // problems (the reference's hyperlink layer is above them).
    if let Some(range) = extras.link_highlight {
        let style = link_style(theme, default);
        out = map_range(out, byte_of(range.start)..byte_of(range.end), |r| {
            apply_inline(r, &style)
        });
    }
    // Underline the input method's composition.
    if let Some(marked) = marked {
        let plain = UnderlineStyle {
            color: None,
            thickness: px(1.0),
            wavy: false,
        };
        out = underline_range(out, byte_of(marked.start)..byte_of(marked.end), plain);
    }
    out
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme = cx.theme();
        let (ui_font, transparent) = {
            let e = self.editor.read(cx);
            (e.ui_font, e.transparent)
        };
        let m = metrics(theme, window, ui_font);
        let mut colors = Colors {
            background: theme.color(UiRole::EditorBackground),
            gutter: theme.color(UiRole::Gutter),
            current_line: theme.color(UiRole::CurrentLine),
            selection: theme.color(UiRole::EditorSelection),
            caret: theme.color(UiRole::Caret),
            scrollbar: theme.color(UiRole::Scrollbar),
            scrollbar_hover: theme.color(UiRole::ScrollbarHover),
            search_match: theme.color(UiRole::SearchMatch),
            search_stripe: theme.color(UiRole::SearchMatchStripe),
            search_current: theme.color(UiRole::Caret),
            search_current_background: None,
            text: theme.color(UiRole::EditorText),
        };
        let input_color = theme.color(UiRole::Input);
        let number_color = theme.color(UiRole::LineNumber);
        let number_active = theme.color(UiRole::LineNumberActive);
        let mut text_color = theme.color(UiRole::EditorText);
        let caret_width = theme.metric(Metric::CaretWidth);
        let scrollbar_width = theme.metric(Metric::ScrollbarWidth);
        let lh = m.line_height;

        // Gutter: line numbers right-aligned in a column wide enough for the
        // largest number.
        let (len_lines, tab_size, single_line, error, console) = {
            let e = self.editor.read(cx);
            (
                e.buffer.len_lines(),
                e.config().tab_size,
                e.single_line,
                e.error,
                e.console,
            )
        };
        if single_line {
            colors.background = input_color;
            colors.gutter = input_color;
        }
        if console {
            colors.background = theme.color(UiRole::TerminalBackground);
            colors.gutter = colors.background;
            text_color = theme.color(UiRole::TerminalForeground);
        }
        if transparent {
            colors.background = gpui::transparent_black();
            colors.gutter = gpui::transparent_black();
        }
        if error {
            text_color = theme.color(UiRole::SearchFieldError);
        }
        colors.text = text_color;
        let digits = len_lines.to_string().len().max(2);
        let (gutter_width, text_left) = if single_line {
            (px(0.0), px(0.0))
        } else if console {
            (px(CONSOLE_GUTTER), px(0.0))
        } else {
            (
                px(GUTTER_LEFT + GUTTER_RIGHT) + m.number_width * digits as f32,
                px(TEXT_LEFT),
            )
        };
        let text_x = bounds.left() + gutter_width + text_left;
        let text_width = (bounds.right() - text_x).max(px(0.0));
        let viewport_lines = (bounds.size.height / lh).floor().max(1.0) as usize;

        // Keep the caret in view after edits and caret moves.
        self.editor.update(cx, |e, _| {
            let max_y = if console {
                (lh * len_lines as f32 - bounds.size.height).max(px(0.0))
            } else {
                lh * len_lines.saturating_sub(1) as f32
            };
            if std::mem::take(&mut e.scroll_to_end) {
                e.scroll.y = max_y;
            }
            if let Some(pos) = e.reveal.take() {
                // A search match goes to the middle of the view when hidden.
                let y = lh * e.buffer.char_to_line(pos) as f32;
                if y < e.scroll.y || y + lh > e.scroll.y + bounds.size.height {
                    e.scroll.y = y - (bounds.size.height - lh) / 2.0;
                }
            }
            if e.autoscroll {
                e.autoscroll = false;
                let head = e.selections.primary().head;
                let p = e.buffer.offset_to_point(head);
                let y = lh * p.line as f32;
                if y < e.scroll.y {
                    e.scroll.y = y;
                } else if y + lh > e.scroll.y + bounds.size.height {
                    e.scroll.y = y + lh - bounds.size.height;
                }
                let dcol = movement::display_column(&e.buffer, p.line, p.column, tab_size);
                let x = m.char_width * dcol as f32;
                let margin = m.char_width * 4.0;
                if x < e.scroll.x {
                    e.scroll.x = (x - margin).max(px(0.0));
                } else if x + margin > e.scroll.x + text_width {
                    e.scroll.x = x + margin - text_width;
                }
            }
            if e.center_caret {
                // Go to Line: the caret line in the middle of the view.
                e.center_caret = false;
                let line = e.buffer.char_to_line(e.selections.primary().head);
                e.scroll.y = lh * line as f32 - (bounds.size.height - lh) / 2.0;
            }
            e.scroll.y = e.scroll.y.clamp(px(0.0), max_y);
            e.scroll.x = e.scroll.x.max(px(0.0));
            // Highlight spans for the lines about to be painted.
            let first = ((e.scroll.y / lh).floor() as usize).min(len_lines - 1);
            let last = (first + viewport_lines + 2).min(len_lines);
            if let Some(h) = &mut e.highlighter {
                h.prepare(&e.buffer, first..last);
            }
        });

        let editor = self.editor.read(cx);
        let theme = cx.theme();
        let scroll = editor.scroll;
        let first = ((scroll.y / lh).floor() as usize).min(len_lines - 1);
        let last = (first + viewport_lines + 2).min(len_lines);
        let visible = first..last;

        // Shape the visible lines.
        let mut spans = Vec::new();
        let mut errors = Vec::new();
        let mut styles = Vec::new();
        let mut line_links = Vec::new();
        // Hyperlinks: (line, char range).
        let mut links: Vec<(usize, Range<usize>)> = Vec::new();
        let mut lines = Vec::with_capacity(visible.len());
        let visible_cols = (text_width / m.char_width).ceil() as usize + 1;
        let scroll_col = (scroll.x / m.char_width).floor() as usize;
        // Problems the text runs do not draw: dotted underlines (line,
        // char columns, colour) and the looks after line ends.
        let mut line_dotted: Vec<(usize, Range<usize>, Hsla)> = Vec::new();
        let mut line_after_end: Vec<(usize, Look)> = Vec::new();
        // The link highlight's parts: (line, char columns).
        let mut link_highlight_cols: Vec<(usize, Range<usize>)> = Vec::new();
        for line in visible.clone() {
            let len = editor.buffer.line_len(line);
            let cols = if len > LONG_LINE {
                let start = scroll_col.saturating_sub(LONG_LINE_MARGIN).min(len);
                start..(scroll_col + visible_cols + LONG_LINE_MARGIN).min(len)
            } else {
                0..len
            };
            let (text, col_bytes) = display_text(editor, line, cols.clone(), tab_size);
            spans.clear();
            errors.clear();
            styles.clear();
            line_links.clear();
            if let Some(h) = editor.highlighter() {
                h.line_spans(line, &mut spans);
                h.line_errors(line, &mut errors);
                h.line_styles(line, theme, &mut styles);
                h.line_links(line, &mut line_links);
            }
            links.extend(line_links.iter().map(|r| (line, r.clone())));
            let line_start = editor.buffer.line_start(line);
            let marks = &editor.diagnostics;
            let looks = line_looks(
                marks
                    .near(line_start..line_start + len)
                    .map(|i| marks.get(i)),
                line_start,
                len,
                theme,
            );
            for (range, look) in &looks.ranges {
                if let Some(color) = look.dotted(text_color) {
                    line_dotted.push((line, range.clone(), color));
                }
            }
            if let Some(look) = looks.after_end {
                line_after_end.push((line, look));
            }
            let line_end = line_start + len;
            let marked = editor.marked.as_ref().and_then(|r| {
                (r.start < line_end.max(line_start + 1) && r.end > line_start)
                    .then(|| r.start.max(line_start) - line_start..r.end.min(line_end) - line_start)
            });
            let link_highlight = editor.link_highlight.as_ref().and_then(|r| {
                (r.start < line_end && r.end > line_start)
                    .then(|| r.start.max(line_start) - line_start..r.end.min(line_end) - line_start)
            });
            if let Some(cols) = &link_highlight {
                link_highlight_cols.push((line, cols.clone()));
            }
            let text_runs = runs(
                &spans,
                LineExtras {
                    errors: &errors,
                    styles: &styles,
                    links: &line_links,
                    diagnostics: &looks.ranges,
                    link_highlight,
                },
                &cols,
                col_bytes.as_ref(),
                text.len(),
                &m.font,
                text_color,
                theme,
                marked,
            );
            let shaped = window.text_system().shape_line(
                SharedString::from(text),
                m.font_size,
                &text_runs,
                None,
            );
            lines.push(LineLayout {
                line,
                start_col: cols.start,
                x_offset: m.char_width * cols.start as f32,
                shaped,
                col_bytes,
            });
        }

        let top_of = |line: usize| bounds.top() + lh * line as f32 - scroll.y;
        let line_x = |l: &LineLayout, col: usize| text_x - scroll.x + l.x_for_col(col);
        let layout_of = |line: usize| lines.get(line.wrapping_sub(first));

        let (underline_offset, strike_offset) = m.effect_offsets();
        let dotted: Vec<Dotted> = line_dotted
            .into_iter()
            .filter_map(|(line, range, color)| {
                let l = layout_of(line)?;
                Some(Dotted {
                    x0: line_x(l, range.start),
                    x1: line_x(l, range.end),
                    y: top_of(line) + underline_offset,
                    color,
                })
            })
            .collect();
        let after_end: Vec<AfterEnd> = line_after_end
            .into_iter()
            .filter_map(|(line, look)| {
                let l = layout_of(line)?;
                let x = line_x(l, editor.buffer.line_len(line));
                Some(AfterEnd {
                    cell: Bounds::new(point(x, top_of(line)), size(m.char_width, lh)),
                    underline_y: top_of(line) + underline_offset,
                    strike_y: top_of(line) + strike_offset,
                    look,
                    text: text_color,
                })
            })
            .collect();

        // Selections: up to the end of the text on the last line, the full
        // width on lines whose line break is selected.
        let mut selection_rects = Vec::new();
        let mut caret_lines = Vec::new();
        let mut caret = None;
        let focused = editor.focus_handle_ref().is_focused(window);
        for s in editor.selections.iter() {
            let head = editor.buffer.offset_to_point(s.head);
            caret_lines.push(head.line);
            if focused
                && editor.caret_visible
                && s.head == editor.selections.primary().head
                && let Some(l) = layout_of(head.line)
            {
                caret = Some(Bounds::new(
                    point(line_x(l, head.column), top_of(head.line)),
                    size(caret_width, lh),
                ));
            }
            if s.is_empty() {
                continue;
            }
            let a = editor.buffer.offset_to_point(s.start());
            let b = editor.buffer.offset_to_point(s.end());
            for line in a.line.max(first)..=b.line.min(last.saturating_sub(1)) {
                let Some(l) = layout_of(line) else { continue };
                let x0 = if line == a.line {
                    line_x(l, a.column)
                } else {
                    text_x - scroll.x
                };
                let x1 = if line == b.line {
                    line_x(l, b.column)
                } else {
                    bounds.right()
                };
                if x1 > x0 {
                    selection_rects.push(Bounds::from_corners(
                        point(x0, top_of(line)),
                        point(x1, top_of(line) + lh),
                    ));
                }
            }
        }
        // Search matches on the visible lines.
        let mut search_rects = Vec::new();
        if let Some(h) = &editor.search {
            if h.current_background {
                colors.search_current_background = Some(theme.color(UiRole::SearchMatchActive));
            }
            let vis_start = editor.buffer.line_start(first);
            let vis_end = editor.buffer.line_end(last.saturating_sub(1).max(first));
            let from = h.first_ending_at_or_after(vis_start);
            for (i, r) in h.matches.iter().enumerate().skip(from) {
                if r.start > vis_end {
                    break;
                }
                let a = editor.buffer.offset_to_point(r.start);
                let b = editor.buffer.offset_to_point(r.end);
                for line in a.line.max(first)..=b.line.min(last.saturating_sub(1)) {
                    let Some(l) = layout_of(line) else { continue };
                    let x0 = if line == a.line {
                        line_x(l, a.column)
                    } else {
                        text_x - scroll.x
                    };
                    let x1 = if line == b.line {
                        line_x(l, b.column)
                    } else {
                        // The line break of a match that goes on.
                        line_x(l, editor.buffer.line_len(line)) + m.char_width
                    };
                    // An empty match is a thin bar.
                    let x1 = if x1 > x0 { x1 } else { x0 + px(2.0) };
                    search_rects.push(SearchRect {
                        bounds: Bounds::from_corners(
                            point(x0, top_of(line)),
                            point(x1, top_of(line) + lh),
                        ),
                        current: h.current == Some(i),
                        excluded: h.excluded.contains(&i),
                    });
                }
            }
        }

        // The placeholder of an empty one-line field.
        let placeholder = editor.shown_placeholder(focused).cloned().map(|text| {
            let run = TextRun {
                len: text.len(),
                font: m.font.clone(),
                color: theme.color(UiRole::TextMuted),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window
                .text_system()
                .shape_line(text, m.font_size, &[run], None);
            (shaped, point(text_x - scroll.x, top_of(0)))
        });

        // Hyperlink rectangles on the visible lines.
        let link_rects: Vec<LinkRect> = links
            .into_iter()
            .filter_map(|(line, range)| {
                let l = layout_of(line)?;
                let x0 = line_x(l, range.start);
                let x1 = line_x(l, range.end);
                (x1 > x0).then(|| LinkRect {
                    bounds: Bounds::from_corners(
                        point(x0, top_of(line)),
                        point(x1, top_of(line) + lh),
                    ),
                    line,
                    range,
                })
            })
            .collect();

        // The caret line is highlighted when nothing is selected (not in a
        // console).
        let current_lines = if !single_line
            && !console
            && editor.selections.iter().all(|s| s.is_empty())
        {
            caret_lines
                .iter()
                .filter(|l| visible.contains(l))
                .map(|&l| Bounds::new(point(bounds.left(), top_of(l)), size(bounds.size.width, lh)))
                .collect()
        } else {
            Vec::new()
        };

        // Line numbers, right-aligned.
        let numbers_right = bounds.left() + px(GUTTER_LEFT) + m.number_width * digits as f32;
        let numbers = visible
            .clone()
            .filter(|_| !single_line && !console)
            .map(|line| {
                let label = SharedString::from((line + 1).to_string());
                let color = if caret_lines.contains(&line) {
                    number_active
                } else {
                    number_color
                };
                let run = TextRun {
                    len: label.len(),
                    font: m.font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window
                    .text_system()
                    .shape_line(label, m.number_size, &[run], None);
                let origin = point(numbers_right - shaped.width, top_of(line));
                (shaped, origin)
            })
            .collect();

        // Vertical scrollbar when the text is taller than the view; the view
        // can scroll until the last line is at the top.
        let content = if console {
            (lh * len_lines as f32).max(bounds.size.height)
        } else {
            lh * (len_lines.saturating_sub(1)) as f32 + bounds.size.height
        };
        let scrollbar = (!single_line && len_lines > viewport_lines).then(|| {
            let track = Bounds::new(
                point(bounds.right() - scrollbar_width, bounds.top()),
                size(scrollbar_width, bounds.size.height),
            );
            let h = (bounds.size.height * (bounds.size.height / content))
                .max(px(MIN_THUMB))
                .min(bounds.size.height);
            let max_y = content - bounds.size.height;
            let ratio = if max_y > px(0.0) {
                scroll.y / max_y
            } else {
                0.0
            };
            let y = bounds.top() + (bounds.size.height - h) * ratio;
            let thumb = Bounds::new(
                point(track.left() + px(2.0), y),
                size(scrollbar_width - px(4.0), h),
            );
            Scrollbar { track, thumb }
        });

        // The error stripe: the scrollbar's track, or the same strip when
        // the text does not scroll but problems are shown (as the
        // reference's editor always has its error stripe).
        let shows_problems = editor.analysis != Analysis::Off || !editor.diagnostics().is_empty();
        let stripe = scrollbar.map(|s| s.track).or_else(|| {
            (!single_line && !console && shows_problems).then(|| {
                Bounds::new(
                    point(bounds.right() - scrollbar_width, bounds.top()),
                    size(scrollbar_width, bounds.size.height),
                )
            })
        });
        // A line's y on the stripe: where it is in the view while the whole
        // text fits, else in proportion (the reference's error stripe).
        let stripe_y = |track: &Bounds<Pixels>, line: usize| -> Pixels {
            let y = lh * line as f32;
            let target = track.size.height;
            let y = if content <= target {
                y
            } else {
                y * (target / content)
            };
            (track.top() + y).floor()
        };
        let mark_bounds = |track: &Bounds<Pixels>, top: Pixels, bottom: Pixels| {
            Bounds::from_corners(
                point(track.left() + px(2.0), top),
                point(track.right() - px(2.0), bottom.max(top + px(STRIPE_MARK))),
            )
        };

        // Search matches on the stripe: a mark per line, drawn once per
        // pixel row.
        let mut stripe_marks = Vec::new();
        if let (Some(track), Some(h)) = (&stripe, &editor.search) {
            let mut last_y = None;
            for &line in &h.lines {
                let y = stripe_y(track, line);
                if last_y == Some(y) {
                    continue;
                }
                last_y = Some(y);
                stripe_marks.push(mark_bounds(track, y, y));
            }
        }
        // Problems on the stripe in the severity's stripe colour, from the
        // start line to the end line, the most severe drawn last;
        // information has no mark.
        let mut problem_stripes = Vec::new();
        if let Some(track) = &stripe {
            let marks = editor.diagnostics();
            let lines = editor.diagnostics.lines(&editor.buffer);
            let mut order: Vec<usize> = (0..marks.len())
                .filter(|&i| marks[i].severity != MarkSeverity::Information)
                .collect();
            order.sort_by_key(|&i| std::cmp::Reverse(marks[i].severity));
            for i in order {
                let m = &marks[i];
                let role = match m.severity {
                    MarkSeverity::Error => UiRole::DiagnosticErrorStripe,
                    MarkSeverity::Warning => UiRole::DiagnosticWarningStripe,
                    _ => UiRole::DiagnosticWeakWarningStripe,
                };
                let (first, last) = lines[i];
                let top = stripe_y(track, first);
                let bottom = stripe_y(track, last);
                problem_stripes.push(StripeMark {
                    bounds: mark_bounds(track, top, bottom),
                    color: theme.color(role),
                    mark: i,
                });
            }
        }
        let problem_spans = stripe_spans(stripe.as_ref(), &problem_stripes);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_right = stripe.map_or(bounds.right(), |s| s.left());
        let text_hitbox = window.insert_hitbox(
            Bounds::from_corners(
                point(bounds.left() + gutter_width, bounds.top()),
                point(text_right, bounds.bottom()),
            ),
            HitboxBehavior::Normal,
        );
        let link_hitboxes = link_rects
            .iter()
            .map(|l| {
                let visible = l.bounds.intersect(&Bounds::from_corners(
                    point(bounds.left() + gutter_width, bounds.top()),
                    point(text_right, bounds.bottom()),
                ));
                window.insert_hitbox(visible, HitboxBehavior::Normal)
            })
            .collect();
        // The link highlight: the hand pointer over it.
        let text_area = Bounds::from_corners(
            point(bounds.left() + gutter_width, bounds.top()),
            point(text_right, bounds.bottom()),
        );
        let link_highlight_hitboxes = link_highlight_cols
            .into_iter()
            .filter_map(|(line, cols)| {
                let l = layout_of(line)?;
                let rect = Bounds::from_corners(
                    point(line_x(l, cols.start), top_of(line)),
                    point(line_x(l, cols.end), top_of(line) + lh),
                )
                .intersect(&text_area);
                (rect.size.width > px(0.0))
                    .then(|| window.insert_hitbox(rect, HitboxBehavior::Normal))
            })
            .collect();
        let layout = LayoutInfo {
            bounds,
            text_x,
            line_height: lh,
            char_width: m.char_width,
            visible,
            lines,
            viewport_lines,
            scrollbar,
            stripe,
            problem_stripes,
            links: link_rects,
        };
        let stored = layout.clone();
        self.editor.update(cx, |e, _| e.layout = Some(stored));
        Prepaint {
            hitbox,
            text_hitbox,
            layout,
            gutter_width,
            numbers,
            current_lines,
            selections: selection_rects,
            caret,
            search: search_rects,
            stripe_marks,
            problem_spans,
            placeholder,
            link_hitboxes,
            link_highlight_hitboxes,
            dotted,
            after_end,
            colors,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus_handle_ref().clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let lh = p.layout.line_height;
        let scroll_x = self.editor.read(cx).scroll.x;
        let text_bounds = Bounds::from_corners(
            point(bounds.left() + p.gutter_width, bounds.top()),
            bounds.bottom_right(),
        );
        // Arrow over the gutter and the scrollbar, I-beam over the text.
        ui::pointer::request(CursorStyle::Arrow, &p.hitbox, window);
        ui::pointer::request(CursorStyle::IBeam, &p.text_hitbox, window);
        for link in p.link_hitboxes.iter().chain(&p.link_highlight_hitboxes) {
            ui::pointer::request(CursorStyle::PointingHand, link, window);
        }

        window.paint_quad(fill(bounds, p.colors.background));
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(p.gutter_width, bounds.size.height)),
            p.colors.gutter,
        ));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for r in &p.current_lines {
                window.paint_quad(fill(*r, p.colors.current_line));
            }
            for (shaped, origin) in &p.numbers {
                let _ = shaped.paint(*origin, lh, window, cx);
            }
        });
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                // Search matches go under the selection (the current match
                // is the selection); when searching within a selection they
                // go over it, the current one with the active background.
                let paint_search = |window: &mut Window| {
                    for r in &p.search {
                        let color = match (r.current, p.colors.search_current_background) {
                            (true, Some(active)) => active,
                            _ => p.colors.search_match,
                        };
                        window.paint_quad(fill(r.bounds, color));
                    }
                };
                // Text backgrounds (a console's ANSI backgrounds) go under
                // the search matches and the selection.
                for l in &p.layout.lines {
                    let origin = point(
                        p.layout.text_x - scroll_x + l.x_offset,
                        bounds.top() + lh * l.line as f32 - self.editor.read(cx).scroll.y,
                    );
                    let _ = l.shaped.paint_background(origin, lh, window, cx);
                }
                for a in &p.after_end {
                    if let Some(bg) = a.look.bg {
                        window.paint_quad(fill(a.cell, bg));
                    }
                }
                let in_selection = p.colors.search_current_background.is_some();
                if !in_selection {
                    paint_search(window);
                }
                for r in &p.selections {
                    window.paint_quad(fill(*r, p.colors.selection));
                }
                if in_selection {
                    paint_search(window);
                }
                for l in &p.layout.lines {
                    let origin = point(
                        p.layout.text_x - scroll_x + l.x_offset,
                        bounds.top() + lh * l.line as f32 - self.editor.read(cx).scroll.y,
                    );
                    let _ = l.shaped.paint(origin, lh, window, cx);
                }
                if let Some((shaped, origin)) = &p.placeholder {
                    let _ = shaped.paint(*origin, lh, window, cx);
                }
                for d in &p.dotted {
                    paint_dots(window, d.x0, d.x1, d.y, d.color);
                }
                for a in &p.after_end {
                    paint_after_end(window, a);
                }
                for r in &p.search {
                    if r.excluded {
                        // Excluded from Replace All: struck through.
                        let y = r.bounds.center().y.round();
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                point(r.bounds.left(), y),
                                point(r.bounds.right(), y + px(1.0)),
                            ),
                            p.colors.text,
                        ));
                    }
                    if r.current {
                        window.paint_quad(
                            outline(r.bounds, p.colors.search_current, BorderStyle::Solid)
                                .corner_radii(px(3.0)),
                        );
                    }
                }
                if let Some(caret) = p.caret {
                    window.paint_quad(fill(caret, p.colors.caret));
                }
            },
        );
        for r in &p.stripe_marks {
            window.paint_quad(fill(*r, p.colors.search_stripe));
        }
        for (bounds, color) in &p.problem_spans {
            window.paint_quad(fill(*bounds, *color));
        }
        if let Some(bar) = p.layout.scrollbar {
            let hovered =
                window.mouse_position().x >= bar.track.left() && p.hitbox.is_hovered(window);
            let thumb_color = if hovered {
                p.colors.scrollbar_hover
            } else {
                p.colors.scrollbar
            };
            window
                .paint_quad(fill(bar.thumb, thumb_color).corner_radii(bar.thumb.size.width / 2.0));
        }

        // Mouse input.
        let hitbox = p.hitbox.clone();
        let editor = self.editor.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || e.button != MouseButton::Left || !hitbox.is_hovered(window) {
                return;
            }
            editor.update(cx, |ed, cx| {
                let Some(layout) = ed.layout.clone() else {
                    return;
                };
                if layout.stripe.is_some_and(|s| s.contains(&e.position)) {
                    // A problem's mark is clicked (pressed and released in
                    // place; at once when the thumb is not under it); else
                    // the thumb is dragged or moved here.
                    let mark = layout.stripe_marks_near(e.position.y).first().copied();
                    let on_thumb = layout
                        .scrollbar
                        .is_some_and(|b| b.thumb.contains(&e.position));
                    if let Some(mark) = mark
                        && !on_thumb
                    {
                        ed.stripe_clicked(mark, window, cx);
                        return;
                    }
                    ed.pressed_stripe = mark.map(|m| (m, e.position));
                    if let Some(bar) = layout.scrollbar {
                        let grab = if on_thumb {
                            e.position.y - bar.thumb.top()
                        } else {
                            bar.thumb.size.height / 2.0
                        };
                        ed.drag = Some(DragUnit::Scrollbar(grab));
                        ed.drag_scrollbar(e.position.y, cx);
                    }
                    return;
                }
                // Cmd+click (Ctrl+click) on a char is reported for
                // navigation; the caret stays.
                if e.modifiers.secondary()
                    && e.click_count == 1
                    && let Some(offset) = ed.char_at(e.position)
                {
                    window.focus(ed.focus_handle_ref());
                    cx.emit(EditorEvent::ModifiedClick {
                        offset,
                        modifiers: e.modifiers,
                    });
                    return;
                }
                // A link is followed when the mouse is released where it
                // was pressed (`EditorHyperlinkSupport`).
                ed.pressed_link = ed
                    .layout
                    .as_ref()
                    .and_then(|l| l.link_at(e.position))
                    .filter(|_| e.click_count == 1)
                    .map(|l| (l.line, l.range.clone(), e.position));
                let pos = ed.offset_for_position(e.position);
                ed.mouse_down(pos, e.click_count, e.modifiers.shift, window, cx);
            });
            cx.stop_propagation();
        });
        // Hover: the char under the mouse, the stripe marks under it.
        let editor = self.editor.clone();
        let (text_hitbox, hitbox) = (p.text_hitbox.clone(), p.hitbox.clone());
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
            if !phase.bubble() || e.pressed_button.is_some() {
                return;
            }
            let over_text = text_hitbox.is_hovered(window);
            let over_editor = hitbox.is_hovered(window);
            editor.update(cx, |ed, cx| {
                ed.mouse_moved(e.position, e.modifiers, over_text, over_editor, cx)
            });
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
            if !phase.bubble() || e.pressed_button != Some(MouseButton::Left) {
                return;
            }
            editor.update(cx, |ed, cx| match ed.drag {
                Some(DragUnit::Scrollbar(_)) => ed.drag_scrollbar(e.position.y, cx),
                Some(_) => {
                    // Dragging above or below the view scrolls it.
                    if let Some(l) = &ed.layout {
                        let (top, bottom, lh) = (l.bounds.top(), l.bounds.bottom(), l.line_height);
                        if e.position.y < top {
                            ed.scroll_by(point(px(0.0), -lh), cx);
                        } else if e.position.y > bottom {
                            ed.scroll_by(point(px(0.0), lh), cx);
                        }
                    }
                    let pos = ed.offset_for_position(e.position);
                    ed.mouse_drag(pos, cx);
                }
                None => {}
            });
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && e.button == MouseButton::Left {
                let in_place = |at: gpui::Point<Pixels>| {
                    (e.position.x - at.x).abs() <= px(1.0) && (e.position.y - at.y).abs() <= px(1.0)
                };
                editor.update(cx, |ed, cx| {
                    ed.mouse_up(cx);
                    if let Some((line, range, at)) = ed.pressed_link.take()
                        && in_place(at)
                    {
                        cx.emit(EditorEvent::LinkClicked { line, range });
                    }
                    if let Some((mark, at)) = ed.pressed_stripe.take()
                        && in_place(at)
                    {
                        ed.stripe_clicked(mark, window, cx);
                    }
                });
            }
        });
        // A one-line field does not scroll with the wheel or a swipe, which
        // could move its text out of view; the caret keeps the text in view.
        if !self.editor.read(cx).single_line {
            let editor = self.editor.clone();
            let hitbox = p.hitbox.clone();
            window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let delta = e.delta.pixel_delta(lh);
                editor.update(cx, |ed, cx| ed.scroll_by(point(-delta.x, -delta.y), cx));
                cx.stop_propagation();
            });
        }
    }
}

/// The problems' stripe marks as painted: each pixel row of the stripe in
/// the colour of the last mark over it (the marks come in paint order, the
/// most severe last), merged into runs. A thousand quads cost more of a
/// frame than the rest of the editor.
fn stripe_spans(
    stripe: Option<&Bounds<Pixels>>,
    marks: &[StripeMark],
) -> Vec<(Bounds<Pixels>, Hsla)> {
    let (Some(track), Some(first)) = (stripe, marks.first()) else {
        return Vec::new();
    };
    let top = f32::from(track.top());
    let rows = f32::from(track.size.height).ceil().max(0.0) as usize + STRIPE_MARK as usize;
    let mut colors: Vec<Hsla> = Vec::new();
    let mut row_color: Vec<u8> = vec![u8::MAX; rows];
    for m in marks {
        let index = match colors.iter().position(|c| *c == m.color) {
            Some(i) => i,
            None => {
                colors.push(m.color);
                colors.len() - 1
            }
        };
        let row = |y: Pixels, round: fn(f32) -> f32| {
            (round(f32::from(y) - top).max(0.0) as usize).min(rows)
        };
        let (r0, r1) = (
            row(m.bounds.top(), f32::floor),
            row(m.bounds.bottom(), f32::ceil),
        );
        row_color[r0..r1.max(r0)].fill(index.min(254) as u8);
    }
    let (left, right) = (first.bounds.left(), first.bounds.right());
    let mut spans = Vec::new();
    let mut r = 0;
    while r < rows {
        let c = row_color[r];
        if c == u8::MAX {
            r += 1;
            continue;
        }
        let start = r;
        while r < rows && row_color[r] == c {
            r += 1;
        }
        spans.push((
            Bounds::from_corners(
                point(left, px(top + start as f32)),
                point(right, px(top + r as f32)),
            ),
            colors[c as usize],
        ));
    }
    spans
}

/// The left edges of the dots of a dotted underline from `x0` to `x1`:
/// gaps as large as the dots, on a grid of the window's x so that
/// neighbouring underlines line up (the reference aligns its dots so).
fn dot_lefts(x0: f32, x1: f32) -> impl Iterator<Item = f32> {
    let first = (x0 / DOT).floor() * DOT;
    (0..)
        .map(move |i| first + i as f32 * DOT * 2.0)
        .take_while(move |x| *x < x1)
}

/// A dotted underline from `x0` to `x1` with its top at `y`: round dots.
fn paint_dots(window: &mut Window, x0: Pixels, x1: Pixels, y: Pixels, color: Hsla) {
    let dot = px(DOT);
    for x in dot_lefts(f32::from(x0), f32::from(x1)) {
        let left = px(x).max(x0);
        let right = (px(x) + dot).min(x1);
        if right > left {
            window.paint_quad(
                fill(
                    Bounds::from_corners(point(left, y), point(right, y + dot)),
                    color,
                )
                .corner_radii(dot / 2.0),
            );
        }
    }
}

/// The decoration of a problem after the end of a line: the effects of its
/// look over one char cell (its background is painted with the text's).
fn paint_after_end(window: &mut Window, a: &AfterEnd) {
    let (left, width) = (a.cell.left(), a.cell.size.width);
    let text = a.look.fg.unwrap_or(a.text);
    if let Some(u) = a.look.underline {
        let color = u.color.unwrap_or(text);
        match u.kind {
            ThemeUnderline::None => {}
            ThemeUnderline::Dotted => paint_dots(window, left, left + width, a.underline_y, color),
            kind => window.paint_underline(
                point(left, a.underline_y),
                width,
                &underline_of(kind, color),
            ),
        }
    }
    if a.look.strikethrough {
        window.paint_strikethrough(
            point(left, a.strike_y),
            width,
            &StrikethroughStyle {
                thickness: px(1.0),
                color: Some(text),
            },
        );
    }
}

impl Editor {
    /// Moves the thumb so that the grab point is at `y`.
    pub(crate) fn drag_scrollbar(&mut self, y: Pixels, cx: &mut gpui::Context<Self>) {
        let (Some(DragUnit::Scrollbar(grab)), Some(layout)) =
            (self.drag.clone(), self.layout.clone())
        else {
            return;
        };
        let Some(bar) = layout.scrollbar else { return };
        let free = bar.track.size.height - bar.thumb.size.height;
        if free <= px(0.0) {
            return;
        }
        let ratio = ((y - grab - bar.track.top()) / free).clamp(0.0, 1.0);
        let max_y = self.max_scroll_y();
        self.set_scroll_y(max_y * ratio, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{DiagnosticMark, MarkSeverity};

    fn mark(range: Range<usize>, severity: MarkSeverity) -> DiagnosticMark {
        DiagnosticMark {
            range,
            severity,
            unnecessary: false,
            deprecated: false,
            messages: Vec::new(),
        }
    }

    /// The run that draws byte `at` of a line.
    fn run_at(runs: &[TextRun], at: usize) -> &TextRun {
        let mut start = 0;
        for r in runs {
            if (start..start + r.len).contains(&at) {
                return r;
            }
            start += r.len;
        }
        panic!("no run at {at}");
    }

    /// Runs of `text` (ASCII) highlighted as `spans`, with `marks`.
    fn line_runs(
        text: &str,
        spans: &[(Range<usize>, TextRole)],
        marks: &[DiagnosticMark],
        theme: &Theme,
    ) -> Vec<TextRun> {
        let looks = line_looks(marks, 0, text.len(), theme);
        runs(
            spans,
            LineExtras {
                errors: &[],
                styles: &[],
                links: &[],
                diagnostics: &looks.ranges,
                link_highlight: None,
            },
            &(0..text.len()),
            None,
            text.len(),
            &gpui::font("Mono"),
            gpui::white(),
            theme,
            None,
        )
    }

    fn dark() -> Theme {
        Theme::new(themes::builtin::default_theme(themes::ThemeKind::Dark), &[])
    }

    #[test]
    fn problems_are_drawn_over_the_highlighting() {
        let theme = dark();
        let text = "import os; foo(); old()";
        let unused = DiagnosticMark {
            unnecessary: true,
            ..mark(0..9, MarkSeverity::Warning)
        };
        let error = mark(11..14, MarkSeverity::Error);
        let deprecated = DiagnosticMark {
            deprecated: true,
            ..mark(18..21, MarkSeverity::Warning)
        };
        let keyword = theme.text_color(TextRole::Keyword);
        let runs = line_runs(
            text,
            &[(0..6, TextRole::Keyword), (11..14, TextRole::FunctionCall)],
            &[unused, error, deprecated],
            &theme,
        );
        // Unused code: grey over the keyword colour, no wave.
        let grey = color(theme.text_style(TextRole::DiagnosticUnused).fg.unwrap());
        assert_ne!(grey, keyword);
        for at in [0, 7] {
            let r = run_at(&runs, at);
            assert_eq!(r.color, grey);
            assert!(r.underline.is_none());
            assert!(r.strikethrough.is_none());
        }
        // An error: a wave in the error colour, the highlighting kept.
        let r = run_at(&runs, 12);
        assert_eq!(r.color, theme.text_color(TextRole::FunctionCall));
        let wave = r.underline.expect("a wave");
        assert!(wave.wavy);
        let error_color = theme
            .text_style(TextRole::DiagnosticError)
            .underline_colour()
            .map(color);
        assert_eq!(wave.color, error_color);
        // Deprecated: struck out in the text colour, no wave.
        let r = run_at(&runs, 19);
        assert!(r.underline.is_none());
        assert_eq!(r.strikethrough.and_then(|s| s.color), Some(gpui::white()));
        // Plain text between them is left alone.
        let r = run_at(&runs, 15);
        assert!(r.underline.is_none() && r.strikethrough.is_none());
        assert_eq!(r.color, gpui::white());
    }

    #[test]
    fn warnings_and_weak_warnings_wave_in_their_colours() {
        let theme = dark();
        let runs = line_runs(
            "value == None",
            &[],
            &[
                mark(0..5, MarkSeverity::WeakWarning),
                mark(6..8, MarkSeverity::Warning),
                mark(9..13, MarkSeverity::Information),
            ],
            &theme,
        );
        let wave_color = |at| {
            let u = run_at(&runs, at).underline.expect("a wave");
            assert!(u.wavy);
            u.color
        };
        let role_color = |role| theme.text_style(role).underline_colour().map(color);
        assert_eq!(wave_color(1), role_color(TextRole::DiagnosticInfo));
        assert_eq!(wave_color(7), role_color(TextRole::DiagnosticWarning));
        assert_ne!(wave_color(1), wave_color(7));
        // Information: a dotted line, painted apart from the runs.
        assert!(run_at(&runs, 10).underline.is_none());
    }

    #[test]
    fn a_light_theme_warning_has_its_background() {
        let theme = Theme::new(
            themes::builtin::default_theme(themes::ThemeKind::Light),
            &[],
        );
        let runs = line_runs(
            "x == None",
            &[],
            &[mark(0..9, MarkSeverity::Warning)],
            &theme,
        );
        let bg = theme.text_style(TextRole::DiagnosticWarning).bg.map(color);
        assert!(bg.is_some());
        assert_eq!(run_at(&runs, 3).background_color, bg);
    }

    #[test]
    fn stripe_marks_are_painted_as_runs_of_rows_the_last_on_top() {
        let track = Bounds::new(point(px(100.0), px(10.0)), size(px(10.0), px(100.0)));
        let mark = |top: f32, bottom: f32, color: Hsla, mark: usize| StripeMark {
            bounds: Bounds::from_corners(point(px(102.0), px(top)), point(px(108.0), px(bottom))),
            color,
            mark,
        };
        let (warning, error) = (gpui::yellow(), gpui::red());
        // In paint order: two touching warnings, an error over the second.
        let spans = stripe_spans(
            Some(&track),
            &[
                mark(20.0, 22.0, warning, 0),
                mark(22.0, 30.0, warning, 1),
                mark(25.0, 27.0, error, 2),
                mark(50.0, 52.0, error, 3),
            ],
        );
        let rows: Vec<(f32, f32, Hsla)> = spans
            .iter()
            .map(|(b, c)| (f32::from(b.top()), f32::from(b.bottom()), *c))
            .collect();
        assert_eq!(
            rows,
            [
                (20.0, 25.0, warning),
                (25.0, 27.0, error),
                (27.0, 30.0, warning),
                (50.0, 52.0, error),
            ]
        );
        assert!(
            spans
                .iter()
                .all(|(b, _)| b.left() == px(102.0) && b.right() == px(108.0))
        );
        assert!(stripe_spans(None, &[mark(20.0, 22.0, error, 0)]).is_empty());
    }

    #[test]
    fn dotted_underlines_are_painted_apart_from_the_runs() {
        let mut theme = themes::builtin::default_theme(themes::ThemeKind::Dark);
        theme.text[TextRole::DiagnosticHint] = themes::TextStyle {
            underline: Some(ThemeUnderline::Dotted),
            underline_color: Some(themes::Rgba::rgb(0x80, 0x80, 0x80)),
            ..themes::TextStyle::default()
        };
        let theme = Theme::new(theme, &[]);
        let marks = [mark(2..5, MarkSeverity::Information)];
        let looks = line_looks(&marks, 0, 8, &theme);
        assert_eq!(looks.ranges.len(), 1);
        assert!(looks.ranges[0].1.dotted(gpui::white()).is_some());
        let runs = line_runs("abcdefgh", &[], &marks, &theme);
        assert!(run_at(&runs, 3).underline.is_none());
        // Dots of 2 px with 2 px gaps on the window's grid.
        assert_eq!(dot_lefts(3.0, 13.0).collect::<Vec<_>>(), [2.0, 6.0, 10.0]);
        assert_eq!(dot_lefts(4.0, 5.0).collect::<Vec<_>>(), [4.0]);
    }

    #[test]
    fn text_fields_show_characters_as_typed() {
        let features = field_font_features();
        assert_eq!(features.is_calt_enabled(), Some(false));
        assert!(
            features.tag_value_list().contains(&("liga".to_owned(), 0)),
            "{:?}",
            features.tag_value_list()
        );
    }
}
