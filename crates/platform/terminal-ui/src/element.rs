//! The grid element: lays out the cells of a session view, sizes the
//! session to them, and paints the frame the plan describes (ADR 0014):
//! cell backgrounds, text runs placed at `column × cell width`, the
//! cursor, composed text, and the scrollbar; it routes the mouse to the
//! view and registers the view as the window's input handler.
//!
//! Metrics follow the reference (`TerminalUi`, `TerminalEditorFactory`):
//! the cell is the advance of `%` in the terminal font, the line height
//! the font's height times the line spacing (1.0); the main screen has
//! 6 px above and below the grid and room for the command markers at the
//! left; the alternate screen (full-screen programs) starts 4 px from the
//! left edge and has no scrollbar.

use std::sync::OnceLock;

use gpui::{
    App, Bounds, CursorStyle, Element, ElementId, ElementInputHandler, Entity, Focusable as _,
    Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, ShapedLine, SharedString, Size,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window, fill, outline, point, px, relative,
    size,
};
use terminal::TerminalSize;
use themes::{Metric, Rgba, UiRole};
use ui::theme::ThemeExt;

use crate::colors::TerminalColors;
use crate::plan::{self, CursorDraw, GlyphRun, GridPlan, PlanOptions, RunStyle, UnderlineKind};
use crate::view::{Geometry, ScrollbarGeometry, SessionView};

/// Space above and below the grid of the main screen
/// (`TerminalUi.blockTopInset`, `blockBottomInset`).
pub const TOP_INSET: f32 = 6.0;
pub const BOTTOM_INSET: f32 = 6.0;
/// Space left of the grid of the main screen: the failed-command marker
/// (3 px) and its gap to the text (9 px), `TerminalUi.errorLineWidth` and
/// `errorLineRightOffset`.
pub const LEFT_INSET: f32 = 12.0;
/// Space left of the grid of the alternate screen
/// (`TerminalUi.alternateBufferLeftInset`).
pub const ALTERNATE_LEFT_INSET: f32 = 4.0;
/// Shortest scrollbar thumb.
const MIN_THUMB: f32 = 24.0;
/// Thickness of the underline and bar cursors (`LineCursorRenderer`).
const CURSOR_LINE: f32 = 2.0;

/// The terminal font and its cell.
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub font: Font,
    pub font_size: Pixels,
    pub cell: Size<Pixels>,
}

/// The font families on the machine (asked once: the list is long).
fn machine_fonts(window: &Window) -> &'static [String] {
    static FONTS: OnceLock<Vec<String>> = OnceLock::new();
    FONTS.get_or_init(|| window.text_system().all_font_names())
}

/// The terminal font the settings chose (Settings | Tools | Terminal |
/// Font); what is not chosen is the theme's terminal font. The application
/// keeps it up to date with [`set_font`].
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalFont {
    /// Tried before the theme's families.
    pub family: Option<String>,
    /// In points.
    pub size: Option<f32>,
    /// The line height, a multiple of the font's own.
    pub line_spacing: f32,
    /// The cell width, a multiple of the advance of the font.
    pub column_spacing: f32,
    /// The family of the characters the font lacks. `None`: the platform's
    /// own fallback only; no font is added behind the user's back (a Nerd
    /// Font prompt shows boxes until a Nerd Font is chosen).
    pub fallback: Option<String>,
}

impl Default for TerminalFont {
    fn default() -> Self {
        Self {
            family: None,
            size: None,
            line_spacing: 1.0,
            column_spacing: 1.0,
            fallback: None,
        }
    }
}

impl TerminalFont {
    /// The families to try: the chosen one, then the theme's.
    fn families(&self, theme: &[String]) -> Vec<String> {
        match &self.family {
            Some(family) => std::iter::once(family.clone())
                .chain(theme.iter().filter(|f| *f != family).cloned())
                .collect(),
            None => theme.to_vec(),
        }
    }
}

impl gpui::Global for TerminalFont {}

/// Makes `font` the terminal font and redraws the windows.
pub fn set_font(cx: &mut App, font: TerminalFont) {
    if cx.try_global::<TerminalFont>() != Some(&font) {
        cx.set_global(font);
        cx.refresh_windows();
    }
}

thread_local! {
    /// The theme's terminal families and the one found on the machine.
    static FAMILY: std::cell::RefCell<Option<(Vec<String>, SharedString)>> =
        const { std::cell::RefCell::new(None) };
}

/// The first of `families` the machine has.
fn terminal_family(families: &[String], names: &[String]) -> SharedString {
    FAMILY.with(|cache| {
        let mut cache = cache.borrow_mut();
        match &*cache {
            Some((key, family)) if key.as_slice() == families => family.clone(),
            _ => {
                let family = SharedString::from(ui::fonts::pick_family(families, names));
                *cache = Some((families.to_vec(), family.clone()));
                family
            }
        }
    })
}

/// The terminal font and the size of its cell: the chosen font
/// ([`set_font`]) over the theme's.
pub fn cell_metrics(window: &mut Window, cx: &App) -> CellMetrics {
    let fonts = &cx.theme().raw().fonts;
    let chosen = cx.try_global::<TerminalFont>().cloned().unwrap_or_default();
    let families = chosen.families(&fonts.terminal_family);
    let font = Font {
        family: terminal_family(&families, machine_fonts(window)),
        features: FontFeatures::disable_ligatures(),
        fallbacks: chosen
            .fallback
            .as_ref()
            .map(|f| FontFallbacks::from_fonts(vec![f.clone()])),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let font_size = px(chosen.size.unwrap_or(fonts.terminal_size));
    let line_spacing = chosen.line_spacing;
    let ts = window.text_system();
    let id = ts.resolve_font(&font);
    let ascent = f32::from(ts.ascent(id, font_size)).ceil();
    let descent = f32::from(ts.descent(id, font_size)).abs().ceil();
    let line_height = px(((ascent + descent) * line_spacing).ceil().max(1.0));
    // `%` is among the widest characters that are not double width
    // (`getCharSize`).
    let width = ts
        .advance(id, font_size, '%')
        .map(|s| s.width)
        .unwrap_or(font_size * 0.6);
    let width = px((f32::from(width) * chosen.column_spacing).round().max(1.0));
    CellMetrics {
        font,
        font_size,
        cell: size(width, line_height),
    }
}

/// The grid of a view in `bounds`: its size in cells and where cell
/// (0, 0) is.
pub fn grid_layout(
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    alternate: bool,
) -> (TerminalSize, gpui::Point<Pixels>) {
    let width = f32::from(bounds.size.width) - LEFT_INSET;
    let height = f32::from(bounds.size.height) - TOP_INSET - BOTTOM_INSET;
    let columns = (width / f32::from(cell.width)).floor().max(2.0) as u16;
    let rows = (height / f32::from(cell.height)).floor().max(1.0) as u16;
    let size = TerminalSize {
        columns,
        rows,
        cell_width: f32::from(cell.width).round() as u16,
        cell_height: f32::from(cell.height).round() as u16,
    };
    let left = if alternate {
        ALTERNATE_LEFT_INSET
    } else {
        LEFT_INSET
    };
    (
        size,
        point(bounds.left() + px(left), bounds.top() + px(TOP_INSET)),
    )
}

fn hsla(c: Rgba) -> Hsla {
    ui::theme::color(c)
}

pub struct TerminalElement {
    view: Entity<SessionView>,
}

impl TerminalElement {
    pub fn new(view: Entity<SessionView>) -> Self {
        Self { view }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// A shaped run and where it is painted.
struct PaintedText {
    origin: gpui::Point<Pixels>,
    shaped: ShapedLine,
}

enum CursorPaint {
    Fill(Bounds<Pixels>, Hsla),
    Outline(Bounds<Pixels>, Hsla),
}

pub struct Frame {
    hitbox: Hitbox,
    background: Hsla,
    quads: Vec<(Bounds<Pixels>, Hsla)>,
    texts: Vec<PaintedText>,
    cursor: Option<CursorPaint>,
    cursor_text: Option<PaintedText>,
    marked: Option<(Bounds<Pixels>, Hsla, PaintedText)>,
    scrollbar: Option<(ScrollbarGeometry, Hsla, Hsla)>,
    line_height: Pixels,
}

/// Shapes a run for painting at its column.
fn shape(
    run: &GlyphRun,
    metrics: &CellMetrics,
    origin: gpui::Point<Pixels>,
    window: &mut Window,
) -> PaintedText {
    let font = font_for(&metrics.font, &run.style);
    let color = hsla(run.style.color);
    let text_run = TextRun {
        len: run.text.len(),
        font,
        color,
        background_color: None,
        underline: run.style.underline.map(|(kind, c)| UnderlineStyle {
            thickness: px(if kind == UnderlineKind::Double {
                2.0
            } else {
                1.0
            }),
            color: Some(hsla(c)),
            wavy: kind == UnderlineKind::Curly,
        }),
        strikethrough: run.style.strikethrough.then(|| StrikethroughStyle {
            thickness: px(1.0),
            color: Some(color),
        }),
    };
    let cell = metrics.cell;
    let shaped = window.text_system().shape_line(
        SharedString::from(run.text.clone()),
        metrics.font_size,
        &[text_run],
        run.grid.then_some(cell.width),
    );
    PaintedText {
        origin: point(
            origin.x + cell.width * run.column as f32,
            origin.y + cell.height * run.line as f32,
        ),
        shaped,
    }
}

fn font_for(base: &Font, style: &RunStyle) -> Font {
    let mut font = base.clone();
    if style.bold {
        font.weight = FontWeight::BOLD;
    }
    if style.italic {
        font.style = FontStyle::Italic;
    }
    font
}

/// The cells `columns` of `line` as bounds.
fn cells_bounds(
    origin: gpui::Point<Pixels>,
    cell: Size<Pixels>,
    line: usize,
    columns: std::ops::Range<usize>,
) -> Bounds<Pixels> {
    Bounds::from_corners(
        point(
            origin.x + cell.width * columns.start as f32,
            origin.y + cell.height * line as f32,
        ),
        point(
            origin.x + cell.width * columns.end as f32,
            origin.y + cell.height * (line + 1) as f32,
        ),
    )
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Frame;

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
    ) -> Frame {
        let metrics = cell_metrics(window, cx);
        let theme = cx.theme();
        let base_colors = TerminalColors::from_theme(theme.raw());
        let scrollbar_width = theme.metric(Metric::ScrollbarWidth);
        let scrollbar_colors = (
            theme.color(UiRole::Scrollbar),
            theme.color(UiRole::ScrollbarHover),
        );
        let focused = self.view.read(cx).is_focused(window);
        let font_id = window.text_system().resolve_font(&metrics.font);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let (plan, geometry, colors, marked) = self.view.update(cx, |view, cx| {
            let alternate = view.snapshot.modes.alt_screen;
            let (grid, _) = grid_layout(bounds, metrics.cell, alternate);
            view.window = window.window_handle();
            view.prepare(grid, &base_colors, cx);
            // The screen may have switched with this snapshot.
            let alternate = view.snapshot.modes.alt_screen;
            let (_, origin) = grid_layout(bounds, metrics.cell, alternate);
            let colors = base_colors.with_overrides(&view.snapshot.color_overrides);
            let key = (metrics.font.family.clone(), metrics.font_size);
            if view.fits_font.as_ref() != Some(&key) {
                view.fits.clear();
                view.fits_font = Some(key);
            }
            let cell_width = f32::from(metrics.cell.width);
            let ts = window.text_system().clone();
            let fits_cache = &mut view.fits;
            let mut fits = |c: char| {
                *fits_cache.entry(c).or_insert_with(|| {
                    ts.advance(font_id, metrics.font_size, c)
                        .is_ok_and(|s| (f32::from(s.width) - cell_width).abs() < 0.5)
                })
            };
            let options = PlanOptions {
                focused,
                blink_on: view.blink_on,
            };
            let plan = plan::plan(
                &view.snapshot,
                &colors,
                &mut view.resolver,
                options,
                &mut fits,
            );
            let snapshot = &view.snapshot;
            let scrollbar = (!snapshot.modes.alt_screen && snapshot.history_size > 0).then(|| {
                let track = Bounds::new(
                    point(bounds.right() - scrollbar_width, bounds.top()),
                    size(scrollbar_width, bounds.size.height),
                );
                let total = (snapshot.history_size + snapshot.rows) as f32;
                let height = (track.size.height * (snapshot.rows as f32 / total))
                    .max(px(MIN_THUMB))
                    .min(track.size.height);
                let ratio = 1.0 - snapshot.display_offset as f32 / snapshot.history_size as f32;
                let top = track.top() + (track.size.height - height) * ratio;
                ScrollbarGeometry {
                    track,
                    thumb: Bounds::new(
                        point(track.left() + px(2.0), top),
                        size(scrollbar_width - px(4.0), height),
                    ),
                    history: snapshot.history_size,
                }
            });
            let geometry = Geometry {
                bounds,
                origin,
                cell: metrics.cell,
                columns: snapshot.columns,
                rows: snapshot.rows,
                scrollbar,
            };
            view.set_geometry(geometry);
            view.note_frame();
            (plan, geometry, colors, view.marked.clone())
        });

        let GridPlan {
            backgrounds,
            runs,
            cursor,
        } = plan;
        let origin = geometry.origin;
        let cell = metrics.cell;
        let quads = backgrounds
            .iter()
            .map(|q| {
                (
                    cells_bounds(origin, cell, q.line, q.columns.clone()),
                    hsla(q.color),
                )
            })
            .collect();
        let texts = runs
            .iter()
            .map(|r| shape(r, &metrics, origin, window))
            .collect();
        let mut cursor_paint = None;
        let mut cursor_text = None;
        let mut marked_paint = None;
        if let Some(c) = cursor {
            let b = cells_bounds(origin, cell, c.line, c.column..c.column + c.cells);
            let color = hsla(c.color);
            match &marked {
                // Composed text replaces the cursor: it is drawn there,
                // underlined, on the default background.
                Some(text) if !text.is_empty() => {
                    let run = GlyphRun {
                        line: c.line,
                        column: c.column,
                        cells: text.chars().count(),
                        text: text.clone(),
                        style: RunStyle {
                            color: colors.foreground,
                            bold: false,
                            italic: false,
                            underline: Some((UnderlineKind::Single, colors.foreground)),
                            strikethrough: false,
                        },
                        grid: false,
                    };
                    let painted = shape(&run, &metrics, origin, window);
                    let width = painted.shaped.width.max(cell.width);
                    let back = Bounds::new(b.origin, size(width, cell.height));
                    marked_paint = Some((back, hsla(colors.background), painted));
                }
                _ => {
                    cursor_paint = Some(match c.draw {
                        CursorDraw::Block => CursorPaint::Fill(b, color),
                        CursorDraw::Hollow => CursorPaint::Outline(b, color),
                        CursorDraw::Underline => CursorPaint::Fill(
                            Bounds::from_corners(
                                point(b.left(), b.bottom() - px(CURSOR_LINE)),
                                b.bottom_right(),
                            ),
                            color,
                        ),
                        CursorDraw::Bar => CursorPaint::Fill(
                            Bounds::new(b.origin, size(px(CURSOR_LINE), b.size.height)),
                            color,
                        ),
                    });
                    cursor_text = c.text.as_ref().map(|r| shape(r, &metrics, origin, window));
                }
            }
        }
        Frame {
            hitbox,
            background: hsla(colors.background),
            quads,
            texts,
            cursor: cursor_paint,
            cursor_text,
            marked: marked_paint,
            scrollbar: geometry
                .scrollbar
                .map(|s| (s, scrollbar_colors.0, scrollbar_colors.1)),
            line_height: cell.height,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus_handle(cx);
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        ui::pointer::request(CursorStyle::IBeam, &frame.hitbox, window);

        let lh = frame.line_height;
        window.paint_quad(fill(bounds, frame.background));
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for (b, color) in &frame.quads {
                window.paint_quad(fill(*b, *color));
            }
            for t in &frame.texts {
                let _ = t.shaped.paint(t.origin, lh, window, cx);
            }
            match &frame.cursor {
                Some(CursorPaint::Fill(b, color)) => window.paint_quad(fill(*b, *color)),
                Some(CursorPaint::Outline(b, color)) => {
                    window.paint_quad(outline(*b, *color, gpui::BorderStyle::Solid))
                }
                None => {}
            }
            if let Some(t) = &frame.cursor_text {
                let _ = t.shaped.paint(t.origin, lh, window, cx);
            }
            if let Some((back, color, t)) = &frame.marked {
                window.paint_quad(fill(*back, *color));
                let _ = t.shaped.paint(t.origin, lh, window, cx);
            }
            if let Some((bar, color, hover)) = &frame.scrollbar {
                let hovered = frame.hitbox.is_hovered(window)
                    && window.mouse_position().x >= bar.track.left();
                window.paint_quad(
                    fill(bar.thumb, if hovered { *hover } else { *color })
                        .corner_radii(bar.thumb.size.width / 2.0),
                );
            }
        });

        let view = self.view.clone();
        let hitbox = frame.hitbox.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.is_hovered(window) {
                return;
            }
            view.update(cx, |v, cx| v.mouse_down(e, window, cx));
            cx.stop_propagation();
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
            if phase.bubble() {
                view.update(cx, |v, cx| v.mouse_move(e, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() {
                view.update(cx, |v, cx| v.mouse_up(e, cx));
            }
        });
        let view = self.view.clone();
        let hitbox = frame.hitbox.clone();
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                return;
            }
            view.update(cx, |v, cx| v.scroll_wheel(e, cx));
            cx.stop_propagation();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_layout_keeps_the_insets() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(812.0), px(412.0)));
        let cell = size(px(8.0), px(16.0));
        let (s, origin) = grid_layout(bounds, cell, false);
        // (812 - 12) / 8 = 100 columns, (412 - 12) / 16 = 25 rows.
        assert_eq!((s.columns, s.rows), (100, 25));
        assert_eq!(origin, point(px(22.0), px(26.0)));
        // The alternate screen keeps the size, closer to the left edge.
        let (alt, alt_origin) = grid_layout(bounds, cell, true);
        assert_eq!(alt, s);
        assert_eq!(alt_origin.x, px(14.0));
        // Never below 2 x 1.
        let tiny = Bounds::new(point(px(0.0), px(0.0)), size(px(5.0), px(5.0)));
        let (t, _) = grid_layout(tiny, cell, false);
        assert_eq!((t.columns, t.rows), (2, 1));
    }

    #[test]
    fn the_chosen_family_goes_first_and_no_symbol_font_is_added() {
        let theme = vec!["JetBrains Mono".to_owned(), "Menlo".to_owned()];
        assert_eq!(TerminalFont::default().families(&theme), theme);
        let chosen = TerminalFont {
            family: Some("Menlo".into()),
            ..TerminalFont::default()
        };
        assert_eq!(chosen.families(&theme), ["Menlo", "JetBrains Mono"]);
        // Nothing the machine has is a fallback unless one is chosen.
        assert_eq!(TerminalFont::default().fallback, None);
    }
}
