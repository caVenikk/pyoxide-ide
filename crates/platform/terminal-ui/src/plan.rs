//! What a frame of the grid draws, worked out from a snapshot without the
//! UI toolkit: per visible row the cell backgrounds merged into runs of
//! one colour, the text in runs of one style placed at their columns, and
//! the cursor (ADR 0014).
//!
//! A run of text holds characters that the terminal font draws one cell
//! wide, so its glyphs land on the grid; a wide character (two cells), a
//! character the font lacks (drawn from a fallback font, whose advance
//! differs) and a character with combining marks get a run of their own at
//! their column, so the text after them stays aligned. Blanks between
//! characters of one style stay in the run; blanks at its end are dropped.

use std::ops::Range;

use terminal::{CellFlags, Color, CursorShape, Point, Snapshot, Span};
use themes::Rgba;

use crate::colors::{CellLook, Resolver, TerminalColors, keeps_its_colour};

/// A line under text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderlineKind {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// The look of a run of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunStyle {
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
    pub underline: Option<(UnderlineKind, Rgba)>,
    pub strikethrough: bool,
}

/// Text drawn from a column of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphRun {
    pub line: usize,
    pub column: usize,
    /// Cells the run covers.
    pub cells: usize,
    pub text: String,
    pub style: RunStyle,
    /// One glyph per cell: the glyphs are placed on the grid.
    pub grid: bool,
}

/// A background over cells `columns` of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellQuad {
    pub line: usize,
    pub columns: Range<usize>,
    pub color: Rgba,
}

/// How the cursor is drawn (`TerminalCursorPainter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorDraw {
    /// A filled cell; the text under it takes the cell's background colour.
    Block,
    /// The outline of the cell: an unfocused view, or a program's choice.
    Hollow,
    /// A line at the bottom of the cell.
    Underline,
    /// A line at the left of the cell.
    Bar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorPlan {
    pub line: usize,
    pub column: usize,
    /// 2 on a wide character.
    pub cells: usize,
    pub draw: CursorDraw,
    pub color: Rgba,
    /// The character under a block cursor, in the inverse colour.
    pub text: Option<GlyphRun>,
}

/// One frame of the grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridPlan {
    pub backgrounds: Vec<CellQuad>,
    pub runs: Vec<GlyphRun>,
    pub cursor: Option<CursorPlan>,
}

/// What the view adds to the snapshot.
#[derive(Clone, Copy, Debug)]
pub struct PlanOptions {
    pub focused: bool,
    /// The blinking cursor is in its visible phase (always true for a
    /// cursor that does not blink).
    pub blink_on: bool,
}

/// Search highlight of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    None,
    Match,
    Current,
}

/// The columns of `line` inside a span.
fn span_columns(span: &Span, line: usize, columns: usize) -> Option<Range<usize>> {
    if line < span.start.line || line > span.end.line {
        return None;
    }
    if span.block {
        return Some(span.start.column..(span.end.column + 1).min(columns));
    }
    let start = if line == span.start.line {
        span.start.column
    } else {
        0
    };
    let end = if line == span.end.line {
        (span.end.column + 1).min(columns)
    } else {
        columns
    };
    (start < end).then_some(start..end)
}

fn underline_of(flags: CellFlags) -> Option<UnderlineKind> {
    if flags.contains(CellFlags::UNDERCURL) {
        Some(UnderlineKind::Curly)
    } else if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
        Some(UnderlineKind::Double)
    } else if flags.contains(CellFlags::DOTTED_UNDERLINE) {
        Some(UnderlineKind::Dotted)
    } else if flags.contains(CellFlags::DASHED_UNDERLINE) {
        Some(UnderlineKind::Dashed)
    } else if flags.contains(CellFlags::UNDERLINE) {
        Some(UnderlineKind::Single)
    } else {
        None
    }
}

/// Builds rows' backgrounds and text runs.
struct RowBuilder<'a> {
    out: &'a mut GridPlan,
    line: usize,
    quad: Option<CellQuad>,
    run: Option<GlyphRun>,
    /// Blanks after the open run, kept if a character of its style follows.
    blanks: usize,
}

impl RowBuilder<'_> {
    fn background(&mut self, columns: Range<usize>, color: Option<Rgba>) {
        match (self.quad.as_mut(), color) {
            (Some(q), Some(c)) if q.color == c && q.columns.end == columns.start => {
                q.columns.end = columns.end;
            }
            (_, color) => {
                if let Some(q) = self.quad.take() {
                    self.out.backgrounds.push(q);
                }
                self.quad = color.map(|color| CellQuad {
                    line: self.line,
                    columns,
                    color,
                });
            }
        }
    }

    fn blank(&mut self, cells: usize) {
        if self.run.is_some() {
            self.blanks += cells;
        }
    }

    fn grid_char(&mut self, column: usize, c: char, style: RunStyle) {
        if let Some(run) = self.run.as_mut()
            && run.grid
            && run.style == style
            && run.column + run.cells + self.blanks == column
        {
            run.text.extend(std::iter::repeat_n(' ', self.blanks));
            run.text.push(c);
            run.cells += self.blanks + 1;
            self.blanks = 0;
            return;
        }
        self.flush_run();
        self.run = Some(GlyphRun {
            line: self.line,
            column,
            cells: 1,
            text: c.to_string(),
            style,
            grid: true,
        });
    }

    fn single(&mut self, column: usize, cells: usize, text: String, style: RunStyle) {
        self.flush_run();
        self.out.runs.push(GlyphRun {
            line: self.line,
            column,
            cells,
            text,
            style,
            grid: false,
        });
    }

    fn flush_run(&mut self) {
        self.blanks = 0;
        if let Some(run) = self.run.take() {
            self.out.runs.push(run);
        }
    }

    fn finish(mut self) {
        self.flush_run();
        if let Some(q) = self.quad.take() {
            self.out.backgrounds.push(q);
        }
    }
}

/// The text style of a cell drawn in `color`.
fn run_style(flags: CellFlags, color: Rgba, underline_color: Option<Rgba>) -> RunStyle {
    RunStyle {
        color,
        bold: flags.contains(CellFlags::BOLD),
        italic: flags.contains(CellFlags::ITALIC),
        underline: underline_of(flags).map(|k| (k, underline_color.unwrap_or(color))),
        strikethrough: flags.contains(CellFlags::STRIKEOUT),
    }
}

/// Works out a frame. `fits` tells whether the terminal font draws a
/// character (not ASCII) in one cell; `colors` already holds the program's
/// colour changes.
pub fn plan(
    snapshot: &Snapshot,
    colors: &TerminalColors,
    resolver: &mut Resolver,
    options: PlanOptions,
    fits: &mut dyn FnMut(char) -> bool,
) -> GridPlan {
    let mut out = GridPlan::default();
    let columns = snapshot.columns;
    if columns == 0 || snapshot.cells.len() < columns * snapshot.rows {
        return out;
    }
    let mut matches: Vec<Range<usize>> = Vec::new();
    let mut extras = Extras {
        all: &snapshot.extras,
        next: 0,
    };
    let mut last_look: Option<((Color, Color, CellFlags, bool), CellLook)> = None;
    for line in 0..snapshot.rows {
        let row = snapshot.row(line);
        let selected = snapshot
            .selection
            .as_ref()
            .and_then(|s| span_columns(s, line, columns));
        let current = snapshot
            .current_match
            .as_ref()
            .and_then(|s| span_columns(s, line, columns));
        matches.clear();
        matches.extend(
            snapshot
                .matches
                .iter()
                .filter_map(|s| span_columns(s, line, columns)),
        );
        let mut next_match = 0;
        let found = |column: usize, next_match: &mut usize| -> Found {
            if current.as_ref().is_some_and(|r| r.contains(&column)) {
                return Found::Current;
            }
            while *next_match < matches.len() && matches[*next_match].end <= column {
                *next_match += 1;
            }
            match matches.get(*next_match) {
                Some(r) if r.contains(&column) => Found::Match,
                _ => Found::None,
            }
        };
        let mut b = RowBuilder {
            out: &mut out,
            line,
            quad: None,
            run: None,
            blanks: 0,
        };
        let mut column = 0;
        while column < columns {
            let cell = &row[column];
            if cell.flags.contains(CellFlags::WIDE_SPACER) {
                column += 1;
                continue;
            }
            let cells = if cell.flags.contains(CellFlags::WIDE) && column + 1 < columns {
                2
            } else {
                1
            };
            // Neighbouring cells mostly share their colours and attributes.
            let key = (cell.fg, cell.bg, cell.flags, keeps_its_colour(cell.c));
            let mut look = match last_look {
                Some((k, look)) if k == key => look,
                _ => {
                    let look = resolver.look(colors, cell.fg, cell.bg, cell.flags, cell.c);
                    last_look = Some((key, look));
                    look
                }
            };
            match found(column, &mut next_match) {
                Found::Current => {
                    look = CellLook {
                        text: colors.current_match_text,
                        background: Some(colors.current_match),
                    };
                }
                Found::Match => {
                    look = CellLook {
                        text: colors.search_match_text,
                        background: Some(colors.search_match),
                    };
                }
                Found::None => {}
            }
            if selected.as_ref().is_some_and(|r| r.contains(&column)) {
                let behind = look.background.unwrap_or(colors.background);
                look.background = Some(if colors.selection.is_opaque() {
                    colors.selection
                } else {
                    colors.selection.over(behind)
                });
            }
            b.background(column..column + cells, look.background);

            let extra = extras.at(Point::new(line, column));
            let underline_color = extra
                .and_then(|e| e.underline_color)
                .map(|c| underline_color(colors, c));
            let style = run_style(cell.flags, look.text, underline_color);
            let decorated = style.underline.is_some() || style.strikethrough;
            let hidden = cell.flags.contains(CellFlags::HIDDEN)
                || cell.flags.contains(CellFlags::LEADING_WIDE_SPACER);
            let blank = hidden || cell.c == ' ' || cell.c == '\0' || cell.c == '\t';
            if blank && !decorated {
                b.blank(cells);
                column += cells;
                continue;
            }
            let c = if blank { ' ' } else { cell.c };
            let marks = extra
                .map(|e| e.zerowidth.as_slice())
                .filter(|m| !m.is_empty());
            let simple = cells == 1 && marks.is_none() && (c.is_ascii() || fits(c));
            if simple {
                b.grid_char(column, c, style);
            } else {
                let mut text = c.to_string();
                if let Some(marks) = marks {
                    text.extend(marks);
                }
                b.single(column, cells, text, style);
            }
            column += cells;
        }
        b.finish();
    }
    out.cursor = cursor_plan(snapshot, colors, resolver, options);
    out
}

/// The extra attributes of the cells, looked up in screen order (the
/// snapshot lists them in that order).
struct Extras<'a> {
    all: &'a [terminal::CellExtra],
    next: usize,
}

impl<'a> Extras<'a> {
    fn at(&mut self, point: Point) -> Option<&'a terminal::CellExtra> {
        while self.next < self.all.len() && self.all[self.next].point < point {
            self.next += 1;
        }
        self.all.get(self.next).filter(|e| e.point == point)
    }
}

/// Combining characters drawn over a cell.
fn cell_marks(snapshot: &Snapshot, point: Point) -> Option<&[char]> {
    snapshot
        .extras
        .iter()
        .find(|e| e.point == point)
        .map(|e| e.zerowidth.as_slice())
        .filter(|m| !m.is_empty())
}

/// An underline colour a program set (SGR 58).
fn underline_color(colors: &TerminalColors, color: Color) -> Rgba {
    match color {
        Color::Indexed(i) => colors.text[i as usize],
        Color::Rgb(c) => Rgba::rgb(c.r, c.g, c.b),
        _ => colors.foreground,
    }
}

fn cursor_plan(
    snapshot: &Snapshot,
    colors: &TerminalColors,
    resolver: &mut Resolver,
    options: PlanOptions,
) -> Option<CursorPlan> {
    let cursor = snapshot.cursor?;
    if cursor.shape == CursorShape::Hidden || !snapshot.modes.show_cursor {
        return None;
    }
    if options.focused && cursor.blinking && !options.blink_on {
        return None;
    }
    let Point { line, column } = cursor.point;
    if line >= snapshot.rows || column >= snapshot.columns {
        return None;
    }
    let cell = snapshot.cell(cursor.point);
    let cells = if cell.flags.contains(CellFlags::WIDE) && column + 1 < snapshot.columns {
        2
    } else {
        1
    };
    let draw = match (cursor.shape, options.focused) {
        (CursorShape::Block, true) => CursorDraw::Block,
        (CursorShape::Block | CursorShape::HollowBlock, _) => CursorDraw::Hollow,
        (CursorShape::Underline, _) => CursorDraw::Underline,
        (CursorShape::Beam, _) => CursorDraw::Bar,
        (CursorShape::Hidden, _) => return None,
    };
    // The cursor takes the colour of the text under it, readable on the
    // cell's background (`getCursorColor`); plain cells use the default
    // text colour.
    let look = resolver.look(colors, cell.fg, cell.bg, cell.flags, cell.c);
    let behind = look.background.unwrap_or(colors.background);
    let plain = cell.fg == Color::Foreground
        && cell.bg == Color::Background
        && !cell.flags.intersects(CellFlags::INVERSE | CellFlags::DIM);
    let color = colors.cursor.unwrap_or_else(|| {
        if plain {
            colors.foreground
        } else {
            resolver.contrasted(look.text, behind, colors.min_contrast.max(4.5))
        }
    });
    let visible = !cell.flags.contains(CellFlags::HIDDEN) && cell.c != ' ' && cell.c != '\0';
    let text = (draw == CursorDraw::Block && visible).then(|| {
        let mut text = cell.c.to_string();
        if let Some(marks) = cell_marks(snapshot, cursor.point) {
            text.extend(marks);
        }
        GlyphRun {
            line,
            column,
            cells,
            text,
            style: run_style(cell.flags, behind, None),
            grid: false,
        }
    });
    Some(CursorPlan {
        line,
        column,
        cells,
        draw,
        color,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal::{Cell, Cursor, Modes, Rgb};

    fn colors() -> TerminalColors {
        TerminalColors::from_theme(&themes::builtin::default_theme(themes::ThemeKind::Dark))
    }

    /// A snapshot of `lines` (one char per cell, padded with blanks).
    fn snap(columns: usize, lines: &[&str]) -> Snapshot {
        let mut cells = Vec::new();
        for line in lines {
            let mut row: Vec<Cell> = line
                .chars()
                .map(|c| Cell {
                    c,
                    ..Cell::default()
                })
                .collect();
            row.resize(columns, Cell::default());
            cells.extend(row);
        }
        Snapshot {
            columns,
            rows: lines.len(),
            cells,
            modes: Modes {
                show_cursor: true,
                ..Modes::default()
            },
            ..Snapshot::default()
        }
    }

    fn opts() -> PlanOptions {
        PlanOptions {
            focused: true,
            blink_on: true,
        }
    }

    fn run_plan(s: &Snapshot) -> GridPlan {
        plan(s, &colors(), &mut Resolver::default(), opts(), &mut |c| {
            c != '\u{f115}'
        })
    }

    #[test]
    fn runs_keep_inner_blanks_and_drop_trailing_ones() {
        let s = snap(20, &["ab  cd    ", "   x"]);
        let p = run_plan(&s);
        let texts: Vec<(usize, usize, &str)> = p
            .runs
            .iter()
            .map(|r| (r.line, r.column, r.text.as_str()))
            .collect();
        assert_eq!(texts, [(0, 0, "ab  cd"), (1, 3, "x")]);
        assert!(p.runs.iter().all(|r| r.grid));
        assert_eq!(p.runs[0].cells, 6);
        // Default backgrounds are not painted per cell.
        assert!(p.backgrounds.is_empty());
    }

    #[test]
    fn styles_split_runs_and_backgrounds_merge() {
        let mut s = snap(10, &["abcdef"]);
        for c in &mut s.cells[2..4] {
            c.fg = Color::Indexed(1);
            c.flags = CellFlags::BOLD;
        }
        for c in &mut s.cells[0..5] {
            c.bg = Color::Rgb(Rgb::new(1, 2, 3));
        }
        let p = run_plan(&s);
        let texts: Vec<&str> = p.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["ab", "cd", "ef"]);
        assert!(p.runs[1].style.bold && !p.runs[0].style.bold);
        assert_eq!(
            p.backgrounds,
            [CellQuad {
                line: 0,
                columns: 0..5,
                color: Rgba::rgb(1, 2, 3)
            }]
        );
    }

    #[test]
    fn wide_fallback_and_combining_characters_get_runs_of_their_own() {
        let mut s = snap(12, &["a中  b\u{f115}c"]);
        // `中` is wide: its second cell is a spacer.
        s.cells[1].flags = CellFlags::WIDE;
        s.cells[2] = Cell {
            c: ' ',
            flags: CellFlags::WIDE_SPACER,
            ..Cell::default()
        };
        s.extras.push(terminal::CellExtra {
            point: Point::new(0, 0),
            zerowidth: vec!['\u{301}'],
            underline_color: None,
            hyperlink: None,
        });
        let p = run_plan(&s);
        let runs: Vec<(usize, usize, &str, bool)> = p
            .runs
            .iter()
            .map(|r| (r.column, r.cells, r.text.as_str(), r.grid))
            .collect();
        assert_eq!(
            runs,
            [
                (0, 1, "a\u{301}", false),
                (1, 2, "中", false),
                (4, 1, "b", true),
                (5, 1, "\u{f115}", false),
                (6, 1, "c", true),
            ]
        );
    }

    #[test]
    fn selection_and_matches_colour_cells() {
        let mut s = snap(10, &["find me", "and me"]);
        s.matches = vec![Span {
            start: Point::new(0, 5),
            end: Point::new(0, 6),
            block: false,
        }];
        s.current_match = Some(Span {
            start: Point::new(1, 4),
            end: Point::new(1, 5),
            block: false,
        });
        s.selection = Some(Span {
            start: Point::new(0, 0),
            end: Point::new(0, 3),
            block: false,
        });
        let c = colors();
        let p = run_plan(&s);
        let bg: Vec<(usize, Range<usize>, Rgba)> = p
            .backgrounds
            .iter()
            .map(|q| (q.line, q.columns.clone(), q.color))
            .collect();
        assert_eq!(
            bg,
            [
                (0, 0..4, c.selection),
                (0, 5..7, c.search_match),
                (1, 4..6, c.current_match)
            ]
        );
        let me = p
            .runs
            .iter()
            .find(|r| r.line == 1 && r.column == 4)
            .unwrap();
        assert_eq!(me.style.color, c.current_match_text);
    }

    #[test]
    fn block_selection_covers_a_rectangle() {
        let s = snap(6, &["abcdef", "ghijkl", "mnopqr"]);
        let span = Span {
            start: Point::new(0, 1),
            end: Point::new(2, 2),
            block: true,
        };
        assert_eq!(span_columns(&span, 1, 6), Some(1..3));
        assert_eq!(span_columns(&span, 3, 6), None);
        let text = Span {
            block: false,
            ..span
        };
        assert_eq!(span_columns(&text, 0, 6), Some(1..6));
        assert_eq!(span_columns(&text, 1, 6), Some(0..6));
        assert_eq!(span_columns(&text, 2, 6), Some(0..3));
        let _ = s;
    }

    #[test]
    fn cursor_block_hollow_and_blink() {
        let mut s = snap(5, &["ab"]);
        s.cursor = Some(Cursor {
            point: Point::new(0, 1),
            shape: CursorShape::Block,
            blinking: true,
        });
        let c = colors();
        let p = run_plan(&s);
        let cursor = p.cursor.clone().unwrap();
        assert_eq!(cursor.draw, CursorDraw::Block);
        assert_eq!(cursor.color, c.foreground);
        let under = cursor.text.unwrap();
        assert_eq!(under.text, "b");
        assert_eq!(under.style.color, c.background);
        // Unfocused: hollow, and it does not blink.
        let unfocused = plan(
            &s,
            &c,
            &mut Resolver::default(),
            PlanOptions {
                focused: false,
                blink_on: false,
            },
            &mut |_| true,
        );
        let hollow = unfocused.cursor.unwrap();
        assert_eq!(hollow.draw, CursorDraw::Hollow);
        assert!(hollow.text.is_none());
        // Focused in the hidden phase of the blink: not drawn.
        let hidden = plan(
            &s,
            &c,
            &mut Resolver::default(),
            PlanOptions {
                focused: true,
                blink_on: false,
            },
            &mut |_| true,
        );
        assert!(hidden.cursor.is_none());
        // A beam stays a bar without the focus; a hidden cursor is not drawn.
        s.cursor.as_mut().unwrap().shape = CursorShape::Beam;
        let bar = plan(
            &s,
            &c,
            &mut Resolver::default(),
            PlanOptions {
                focused: false,
                blink_on: true,
            },
            &mut |_| true,
        );
        assert_eq!(bar.cursor.unwrap().draw, CursorDraw::Bar);
        s.modes.show_cursor = false;
        assert!(run_plan(&s).cursor.is_none());
    }

    #[test]
    fn underlines_and_hidden_text() {
        let mut s = snap(8, &["ab cd"]);
        for c in &mut s.cells[0..4] {
            c.flags = CellFlags::UNDERLINE;
        }
        s.cells[4].flags = CellFlags::HIDDEN;
        let p = run_plan(&s);
        // The underlined blank stays in the run; the hidden `d` is not drawn.
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.runs[0].text, "ab c");
        assert!(matches!(
            p.runs[0].style.underline,
            Some((UnderlineKind::Single, _))
        ));
    }
}
