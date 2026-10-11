//! The editor's custom element: lays out and paints only the visible lines
//! (virtualized), the gutter with line numbers, the current line, selections,
//! the caret and a scrollbar, and routes mouse input to the editor.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity, Font,
    FontFeatures, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle, Style,
    TextRun, UnderlineStyle, Window, fill, point, px, relative, size,
};
use text::movement;
use themes::{Metric, TextRole, UiRole, UnderlineStyle as ThemeUnderline};
use ui::theme::{Theme, ThemeExt, color};

use crate::editor::{DragUnit, Editor};

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
/// Smallest scrollbar thumb.
const MIN_THUMB: f32 = 24.0;

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
    colors: Colors,
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
}

/// Font metrics of the editor font at the editor size.
struct Metrics {
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
    char_width: Pixels,
    /// Line numbers are one size smaller than the text.
    number_size: Pixels,
    number_width: Pixels,
}

fn metrics(theme: &Theme, window: &Window) -> Metrics {
    let font = Font {
        family: theme.mono_family(),
        features: if theme.raw().fonts.editor_ligatures {
            FontFeatures::default()
        } else {
            FontFeatures::disable_ligatures()
        },
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let font_size = theme.editor_size();
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
        number_size,
        number_width: advance(number_size),
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

/// Text runs for a line from highlight spans (char columns of the line).
#[allow(clippy::too_many_arguments)]
fn runs(
    spans: &[(Range<usize>, TextRole)],
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
    // Underline the input method's composition.
    if let Some(marked) = marked {
        let (ms, me) = (byte_of(marked.start), byte_of(marked.end));
        if ms < me {
            let mut split = Vec::with_capacity(out.len() + 2);
            let mut pos = 0;
            for run in out {
                let (rs, re) = (pos, pos + run.len);
                pos = re;
                for (s, e, under) in [
                    (rs, ms.min(re), false),
                    (ms.max(rs), me.min(re), true),
                    (me.max(rs), re, false),
                ] {
                    if s < e {
                        let mut r = run.clone();
                        r.len = e - s;
                        if under {
                            r.underline = Some(UnderlineStyle {
                                color: Some(r.color),
                                thickness: px(1.0),
                                wavy: false,
                            });
                        }
                        split.push(r);
                    }
                }
            }
            return split;
        }
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
        let m = metrics(theme, window);
        let mut colors = Colors {
            background: theme.color(UiRole::EditorBackground),
            gutter: theme.color(UiRole::Gutter),
            current_line: theme.color(UiRole::CurrentLine),
            selection: theme.color(UiRole::EditorSelection),
            caret: theme.color(UiRole::Caret),
            scrollbar: theme.color(UiRole::Scrollbar),
            scrollbar_hover: theme.color(UiRole::ScrollbarHover),
        };
        let input_color = theme.color(UiRole::Input);
        let number_color = theme.color(UiRole::LineNumber);
        let number_active = theme.color(UiRole::LineNumberActive);
        let text_color = theme.color(UiRole::EditorText);
        let caret_width = theme.metric(Metric::CaretWidth);
        let scrollbar_width = theme.metric(Metric::ScrollbarWidth);
        let lh = m.line_height;

        // Gutter: line numbers right-aligned in a column wide enough for the
        // largest number.
        let (len_lines, tab_size, single_line) = {
            let e = self.editor.read(cx);
            (e.buffer.len_lines(), e.config().tab_size, e.single_line)
        };
        if single_line {
            colors.background = input_color;
            colors.gutter = input_color;
        }
        let digits = len_lines.to_string().len().max(2);
        let (gutter_width, text_left) = if single_line {
            (px(0.0), px(0.0))
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
            let max_y = lh * len_lines.saturating_sub(1) as f32;
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
        let mut lines = Vec::with_capacity(visible.len());
        let visible_cols = (text_width / m.char_width).ceil() as usize + 1;
        let scroll_col = (scroll.x / m.char_width).floor() as usize;
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
            if let Some(h) = editor.highlighter() {
                h.line_spans(line, &mut spans);
            }
            let line_start = editor.buffer.line_start(line);
            let marked = editor.marked.as_ref().and_then(|r| {
                let line_end = line_start + len;
                (r.start < line_end.max(line_start + 1) && r.end > line_start)
                    .then(|| r.start.max(line_start) - line_start..r.end.min(line_end) - line_start)
            });
            let text_runs = runs(
                &spans,
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
        // The caret line is highlighted when nothing is selected.
        let current_lines = if !single_line && editor.selections.iter().all(|s| s.is_empty()) {
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
            .filter(|_| !single_line)
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
        let content = lh * (len_lines.saturating_sub(1)) as f32 + bounds.size.height;
        let scrollbar = (!single_line && len_lines > viewport_lines).then(|| {
            let track = Bounds::new(
                point(bounds.right() - scrollbar_width, bounds.top()),
                size(scrollbar_width, bounds.size.height),
            );
            let h = (bounds.size.height * (bounds.size.height / content))
                .max(px(MIN_THUMB))
                .min(bounds.size.height);
            let max_y = lh * len_lines.saturating_sub(1) as f32;
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

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_right = scrollbar.map_or(bounds.right(), |s| s.track.left());
        let text_hitbox = window.insert_hitbox(
            Bounds::from_corners(
                point(bounds.left() + gutter_width, bounds.top()),
                point(text_right, bounds.bottom()),
            ),
            HitboxBehavior::Normal,
        );
        let layout = LayoutInfo {
            bounds,
            text_x,
            line_height: lh,
            char_width: m.char_width,
            visible,
            lines,
            viewport_lines,
            scrollbar,
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
        window.set_cursor_style(CursorStyle::Arrow, &p.hitbox);
        window.set_cursor_style(CursorStyle::IBeam, &p.text_hitbox);

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
                for r in &p.selections {
                    window.paint_quad(fill(*r, p.colors.selection));
                }
                for l in &p.layout.lines {
                    let origin = point(
                        p.layout.text_x - scroll_x + l.x_offset,
                        bounds.top() + lh * l.line as f32 - self.editor.read(cx).scroll.y,
                    );
                    let _ = l.shaped.paint(origin, lh, window, cx);
                }
                if let Some(caret) = p.caret {
                    window.paint_quad(fill(caret, p.colors.caret));
                }
            },
        );
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
                if let Some(bar) = ed.layout.as_ref().and_then(|l| l.scrollbar)
                    && bar.track.contains(&e.position)
                {
                    let grab = if bar.thumb.contains(&e.position) {
                        e.position.y - bar.thumb.top()
                    } else {
                        bar.thumb.size.height / 2.0
                    };
                    ed.drag = Some(DragUnit::Scrollbar(grab));
                    ed.drag_scrollbar(e.position.y, cx);
                    return;
                }
                let pos = ed.offset_for_position(e.position);
                ed.mouse_down(pos, e.click_count, e.modifiers.shift, window, cx);
            });
            cx.stop_propagation();
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
        window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() && e.button == MouseButton::Left {
                editor.update(cx, |ed, cx| ed.mouse_up(cx));
            }
        });
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
