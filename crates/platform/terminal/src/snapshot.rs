//! What the renderer paints: a copy of the visible cells, the cursor, the
//! modes, the selection and the search matches, taken under the terminal's
//! lock (ADR 0014). The types are this crate's own; the emulator's types do
//! not leave the crate.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as GridPoint};
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor};

/// A 24-bit colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// The colour of a cell's text or background, resolved by the renderer
/// against the theme's terminal colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// The palette: the 16 ANSI colours (0 to 15) and the 256-colour cube
    /// and grey ramp (16 to 255).
    Indexed(u8),
    /// A 24-bit colour set by the program.
    Rgb(Rgb),
    /// The default text colour.
    Foreground,
    /// The default background.
    Background,
    /// The cursor colour.
    Cursor,
    /// The bright variant of the default text colour.
    BrightForeground,
    /// The dim variant of the default text colour.
    DimForeground,
    /// The dim variant of ANSI colour 0 to 7.
    Dim(u8),
}

impl Color {
    pub(crate) fn from_term(color: ansi::Color) -> Self {
        match color {
            ansi::Color::Spec(rgb) => Color::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b)),
            ansi::Color::Indexed(index) => Color::Indexed(index),
            ansi::Color::Named(named) => Self::from_named(named),
        }
    }

    fn from_named(named: NamedColor) -> Self {
        let index = named as usize;
        match named {
            NamedColor::Foreground => Color::Foreground,
            NamedColor::Background => Color::Background,
            NamedColor::Cursor => Color::Cursor,
            NamedColor::BrightForeground => Color::BrightForeground,
            NamedColor::DimForeground => Color::DimForeground,
            _ if index < 16 => Color::Indexed(index as u8),
            // DimBlack .. DimWhite follow Cursor.
            _ => Color::Dim((index - NamedColor::DimBlack as usize) as u8),
        }
    }

    #[cfg(test)]
    /// The emulator's colour table index of this colour, if it has one
    /// (`Rgb` has none).
    pub(crate) fn table_index(self) -> Option<usize> {
        Some(match self {
            Color::Indexed(index) => index as usize,
            Color::Rgb(_) => return None,
            Color::Foreground => NamedColor::Foreground as usize,
            Color::Background => NamedColor::Background as usize,
            Color::Cursor => NamedColor::Cursor as usize,
            Color::BrightForeground => NamedColor::BrightForeground as usize,
            Color::DimForeground => NamedColor::DimForeground as usize,
            Color::Dim(index) => NamedColor::DimBlack as usize + index.min(7) as usize,
        })
    }

    pub(crate) fn from_table_index(index: usize) -> Option<Self> {
        if index < 256 {
            return Some(Color::Indexed(index as u8));
        }
        let named = [
            NamedColor::Foreground,
            NamedColor::Background,
            NamedColor::Cursor,
            NamedColor::DimBlack,
            NamedColor::DimRed,
            NamedColor::DimGreen,
            NamedColor::DimYellow,
            NamedColor::DimBlue,
            NamedColor::DimMagenta,
            NamedColor::DimCyan,
            NamedColor::DimWhite,
            NamedColor::BrightForeground,
            NamedColor::DimForeground,
        ];
        named
            .into_iter()
            .find(|n| *n as usize == index)
            .map(Self::from_named)
    }
}

/// Text attributes of a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellFlags(u16);

impl CellFlags {
    pub const BOLD: Self = Self(1);
    pub const ITALIC: Self = Self(1 << 1);
    pub const UNDERLINE: Self = Self(1 << 2);
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 3);
    pub const UNDERCURL: Self = Self(1 << 4);
    pub const DOTTED_UNDERLINE: Self = Self(1 << 5);
    pub const DASHED_UNDERLINE: Self = Self(1 << 6);
    pub const STRIKEOUT: Self = Self(1 << 7);
    /// Swap the text and background colours.
    pub const INVERSE: Self = Self(1 << 8);
    pub const DIM: Self = Self(1 << 9);
    /// Invisible text (the cell keeps its background).
    pub const HIDDEN: Self = Self(1 << 10);
    /// The first cell of a character two cells wide.
    pub const WIDE: Self = Self(1 << 11);
    /// The second cell of a wide character: paint nothing.
    pub const WIDE_SPACER: Self = Self(1 << 12);
    /// An empty last cell of a row, left because the wide character that
    /// follows did not fit: paint nothing.
    pub const LEADING_WIDE_SPACER: Self = Self(1 << 13);
    /// The row continues on the next row (soft wrap).
    pub const WRAPLINE: Self = Self(1 << 14);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    fn from_term(flags: Flags) -> Self {
        const MAP: [(Flags, CellFlags); 15] = [
            (Flags::BOLD, CellFlags::BOLD),
            (Flags::ITALIC, CellFlags::ITALIC),
            (Flags::UNDERLINE, CellFlags::UNDERLINE),
            (Flags::DOUBLE_UNDERLINE, CellFlags::DOUBLE_UNDERLINE),
            (Flags::UNDERCURL, CellFlags::UNDERCURL),
            (Flags::DOTTED_UNDERLINE, CellFlags::DOTTED_UNDERLINE),
            (Flags::DASHED_UNDERLINE, CellFlags::DASHED_UNDERLINE),
            (Flags::STRIKEOUT, CellFlags::STRIKEOUT),
            (Flags::INVERSE, CellFlags::INVERSE),
            (Flags::DIM, CellFlags::DIM),
            (Flags::HIDDEN, CellFlags::HIDDEN),
            (Flags::WIDE_CHAR, CellFlags::WIDE),
            (Flags::WIDE_CHAR_SPACER, CellFlags::WIDE_SPACER),
            (
                Flags::LEADING_WIDE_CHAR_SPACER,
                CellFlags::LEADING_WIDE_SPACER,
            ),
            (Flags::WRAPLINE, CellFlags::WRAPLINE),
        ];
        let mut out = CellFlags::empty();
        if flags.is_empty() {
            return out;
        }
        for (from, to) in MAP {
            if flags.contains(from) {
                out.insert(to);
            }
        }
        out
    }
}

impl std::ops::BitOr for CellFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// One cell of the visible screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color::Foreground,
            bg: Color::Background,
            flags: CellFlags::empty(),
        }
    }
}

/// An OSC 8 hyperlink on a cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub id: String,
    pub uri: String,
}

/// The rarely used attributes of a cell, kept apart so that [`Cell`] stays
/// small and `Copy`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellExtra {
    pub point: Point,
    /// Combining characters drawn over the cell's character.
    pub zerowidth: Vec<char>,
    pub underline_color: Option<Color>,
    pub hyperlink: Option<Hyperlink>,
}

/// A cell position on the visible screen: `line` 0 is the top row of the
/// viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub line: usize,
    pub column: usize,
}

impl Point {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// A range of cells on the visible screen, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: Point,
    pub end: Point,
    /// A rectangle (columns `start.column ..= end.column` on every line)
    /// instead of running text.
    pub block: bool,
}

impl Span {
    /// The cell is inside the span.
    pub fn contains(&self, point: Point) -> bool {
        if point.line < self.start.line || point.line > self.end.line {
            return false;
        }
        if self.block {
            return point.column >= self.start.column && point.column <= self.end.column;
        }
        (point.line > self.start.line || point.column >= self.start.column)
            && (point.line < self.end.line || point.column <= self.end.column)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

/// The cursor on the visible screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub point: Point,
    /// The shape the program asked for; the renderer draws a hollow block
    /// when the view is not focused.
    pub shape: CursorShape,
}

/// Mouse reporting requested by the program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseMode {
    /// No reporting: the mouse selects text.
    #[default]
    None,
    /// Presses and releases (mode 1000).
    Click,
    /// Presses, releases and motion with a button held (mode 1002).
    Drag,
    /// Presses, releases and all motion (mode 1003).
    Motion,
}

/// Terminal modes that change how input is encoded and how the screen is
/// drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modes {
    pub show_cursor: bool,
    /// Cursor keys send `ESC O` sequences (DECCKM).
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse: MouseMode,
    /// Mouse reports in the SGR encoding (mode 1006).
    pub sgr_mouse: bool,
    /// Focus in and out reports (mode 1004).
    pub focus_reporting: bool,
    /// The alternate screen (full-screen programs): no scrollback.
    pub alt_screen: bool,
    /// The wheel sends cursor keys on the alternate screen (mode 1007).
    pub alternate_scroll: bool,
    /// Enter sends CR LF (LNM).
    pub line_feed_new_line: bool,
}

impl Modes {
    pub(crate) fn from_term(mode: TermMode) -> Self {
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::Click
        } else {
            MouseMode::None
        };
        Self {
            show_cursor: mode.contains(TermMode::SHOW_CURSOR),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse,
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            line_feed_new_line: mode.contains(TermMode::LINE_FEED_NEW_LINE),
        }
    }
}

/// The visible state of a session at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub columns: usize,
    pub rows: usize,
    /// `rows * columns` cells, row by row from the top of the viewport.
    pub cells: Vec<Cell>,
    /// Combining characters, underline colours and hyperlinks of the cells
    /// that have them.
    pub extras: Vec<CellExtra>,
    /// `None` while the cursor is scrolled out of view.
    pub cursor: Option<Cursor>,
    pub modes: Modes,
    pub selection: Option<Span>,
    /// Search matches on the visible screen (at most [`MAX_VISIBLE_MATCHES`]).
    pub matches: Vec<Span>,
    /// The current search match, when it is on the visible screen.
    pub current_match: Option<Span>,
    /// How many lines the view is scrolled back into the history (0: at
    /// the bottom).
    pub display_offset: usize,
    /// Lines in the scrollback above the screen.
    pub history_size: usize,
    /// Colours the program changed (OSC 4, 10, 11, 12).
    pub color_overrides: Vec<(Color, Rgb)>,
}

/// The most search matches a snapshot lists.
pub const MAX_VISIBLE_MATCHES: usize = 1000;

impl Snapshot {
    /// The cells of one visible row.
    pub fn row(&self, line: usize) -> &[Cell] {
        let start = line * self.columns;
        &self.cells[start..start + self.columns]
    }

    pub fn cell(&self, point: Point) -> &Cell {
        &self.cells[point.line * self.columns + point.column]
    }

    /// The text of one visible row, without trailing blanks; wide
    /// characters once.
    pub fn line_text(&self, line: usize) -> String {
        let mut text: String = self
            .row(line)
            .iter()
            .filter(|c| {
                !c.flags
                    .intersects(CellFlags::WIDE_SPACER | CellFlags::LEADING_WIDE_SPACER)
            })
            .map(|c| c.c)
            .collect();
        text.truncate(text.trim_end().len());
        text
    }

    /// The visible text, one line per row.
    pub fn text(&self) -> String {
        (0..self.rows)
            .map(|line| self.line_text(line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Grid point (lines above the screen are negative) to a viewport point.
pub(crate) fn to_viewport(point: GridPoint, display_offset: usize) -> Option<Point> {
    let line = point.line.0 + display_offset as i32;
    (line >= 0).then(|| Point::new(line as usize, point.column.0))
}

/// Viewport point to a grid point, clamped to the screen.
pub(crate) fn to_grid<T>(term: &Term<T>, point: Point) -> GridPoint {
    let rows = term.screen_lines();
    let columns = term.columns();
    let line = point.line.min(rows.saturating_sub(1)) as i32 - term.grid().display_offset() as i32;
    GridPoint::new(
        Line(line),
        Column(point.column.min(columns.saturating_sub(1))),
    )
}

/// A grid range clipped to the viewport.
pub(crate) fn clip_span<T>(
    term: &Term<T>,
    start: GridPoint,
    end: GridPoint,
    block: bool,
) -> Option<Span> {
    let offset = term.grid().display_offset() as i32;
    let rows = term.screen_lines() as i32;
    let last_column = term.columns().saturating_sub(1);
    let top = Line(-offset);
    let bottom = Line(rows - 1 - offset);
    if end.line < top || start.line > bottom {
        return None;
    }
    let start = if start.line < top {
        GridPoint::new(top, if block { start.column } else { Column(0) })
    } else {
        start
    };
    let end = if end.line > bottom {
        GridPoint::new(
            bottom,
            if block {
                end.column
            } else {
                Column(last_column)
            },
        )
    } else {
        end
    };
    Some(Span {
        start: to_viewport(start, offset as usize)?,
        end: to_viewport(end, offset as usize)?,
        block,
    })
}

/// Copies the visible state of `term` into `out`, reusing its buffers.
pub(crate) fn fill<T: EventListener>(term: &Term<T>, out: &mut Snapshot) {
    let content = term.renderable_content();
    out.columns = term.columns();
    out.rows = term.screen_lines();
    out.display_offset = content.display_offset;
    out.history_size = term.history_size();
    out.modes = Modes::from_term(content.mode);
    out.cells.clear();
    out.cells.reserve(out.columns * out.rows);
    out.extras.clear();
    let offset = content.display_offset;
    for indexed in content.display_iter {
        let cell: &TermCell = indexed.cell;
        out.cells.push(Cell {
            c: cell.c,
            fg: Color::from_term(cell.fg),
            bg: Color::from_term(cell.bg),
            flags: CellFlags::from_term(cell.flags),
        });
        if cell.extra.is_some() {
            let Some(point) = to_viewport(indexed.point, offset) else {
                continue;
            };
            out.extras.push(CellExtra {
                point,
                zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
                underline_color: cell.underline_color().map(Color::from_term),
                hyperlink: cell.hyperlink().map(|h| Hyperlink {
                    id: h.id().to_owned(),
                    uri: h.uri().to_owned(),
                }),
            });
        }
    }
    // A screen that was not fully written yet (should not happen): pad.
    out.cells.resize(out.columns * out.rows, Cell::default());

    let cursor_shape = match content.cursor.shape {
        ansi::CursorShape::Block => CursorShape::Block,
        ansi::CursorShape::Underline => CursorShape::Underline,
        ansi::CursorShape::Beam => CursorShape::Beam,
        ansi::CursorShape::HollowBlock => CursorShape::HollowBlock,
        ansi::CursorShape::Hidden => CursorShape::Hidden,
    };
    out.cursor = to_viewport(content.cursor.point, offset)
        .filter(|p| p.line < out.rows)
        .map(|point| Cursor {
            point,
            shape: cursor_shape,
        });
    out.selection = content
        .selection
        .and_then(|range| clip_span(term, range.start, range.end, range.is_block));

    out.color_overrides.clear();
    for index in 0..alacritty_terminal::term::color::COUNT {
        if let Some(rgb) = content.colors[index]
            && let Some(color) = Color::from_table_index(index)
        {
            out.color_overrides
                .push((color, Rgb::new(rgb.r, rgb.g, rgb.b)));
        }
    }
    out.matches.clear();
    out.current_match = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_colours_map_to_palette_and_defaults() {
        assert_eq!(Color::from_named(NamedColor::Red), Color::Indexed(1));
        assert_eq!(
            Color::from_named(NamedColor::BrightWhite),
            Color::Indexed(15)
        );
        assert_eq!(Color::from_named(NamedColor::Foreground), Color::Foreground);
        assert_eq!(Color::from_named(NamedColor::DimGreen), Color::Dim(2));
        for color in [
            Color::Indexed(200),
            Color::Foreground,
            Color::Background,
            Color::Cursor,
            Color::Dim(5),
            Color::BrightForeground,
            Color::DimForeground,
        ] {
            assert_eq!(
                Color::from_table_index(color.table_index().unwrap()),
                Some(color)
            );
        }
    }

    #[test]
    fn span_contains_text_and_block() {
        let text = Span {
            start: Point::new(1, 5),
            end: Point::new(3, 2),
            block: false,
        };
        assert!(!text.contains(Point::new(1, 4)));
        assert!(text.contains(Point::new(1, 5)));
        assert!(text.contains(Point::new(2, 0)));
        assert!(text.contains(Point::new(3, 2)));
        assert!(!text.contains(Point::new(3, 3)));
        let block = Span {
            block: true,
            ..text
        };
        assert!(!block.contains(Point::new(2, 0)));
        assert!(!block.contains(Point::new(2, 6)));
        let block = Span {
            start: Point::new(1, 2),
            end: Point::new(3, 5),
            block: true,
        };
        assert!(block.contains(Point::new(2, 3)));
    }
}
