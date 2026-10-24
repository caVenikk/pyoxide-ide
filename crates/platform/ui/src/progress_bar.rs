//! The progress bar of background tasks and the painter that draws it.
//!
//! The look is an extension point: the reference lets a theme or a plugin
//! replace the progress bar's UI (`ProgressBarUI`; "Nyan Progress Bar" style
//! plugins do), so the bar is drawn by the [`ProgressBarPainter`] set with
//! [`set_painter`]. Pyoxide ships the reference's look,
//! [`ReferencePainter`] (`DarculaProgressBarUI`): a 4 px stripe with round
//! ends, the done part in the theme's progress colour over the track, and
//! when the amount is not known a gradient between two colours that moves
//! one step every 50 ms and repeats every 800 ms.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Global, Hsla, IntoElement, Pixels, Rgba, Styled, Window, canvas, fill,
    linear_color_stop, linear_gradient, point, px, size,
};
use themes::UiRole;

use crate::theme::ThemeExt;

/// Height of the bar's stripe (`DarculaProgressBarUI.DEFAULT_WIDTH`).
pub const HEIGHT: f32 = 4.0;
/// Time between two frames of the indeterminate animation
/// (`REPAINT_INTERVAL_DEFAULT`).
pub const FRAME: Duration = Duration::from_millis(50);
/// Time of one cycle of the indeterminate animation (`CYCLE_TIME_DEFAULT`).
pub const CYCLE: Duration = Duration::from_millis(800);
/// Frames in one cycle.
pub const FRAMES: u32 = (CYCLE.as_millis() / FRAME.as_millis()) as u32;
/// How far the gradient moves in half a frame.
const STEP: f32 = 6.0;

/// What a bar shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarState {
    /// How much is done, from 0 to 1; `None` (or 0) draws the bar
    /// indeterminate.
    pub fraction: Option<f32>,
    /// The frame of the indeterminate animation, from 0 to [`FRAMES`].
    pub frame: u32,
}

impl BarState {
    pub fn is_indeterminate(&self) -> bool {
        self.fraction.is_none_or(|f| f <= 0.0)
    }
}

/// The frame of the indeterminate animation at `now` of an animation that
/// started at `start` (`updateIndeterminateAnimationIndex`).
pub fn animation_frame(start: Instant, now: Instant) -> u32 {
    let passed = now.saturating_duration_since(start).as_millis() / FRAME.as_millis();
    (passed % u128::from(FRAMES)) as u32
}

/// The theme's colours of a bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarColors {
    /// The part not done yet (`ProgressBar.trackColor`).
    pub track: Hsla,
    /// The part done (`ProgressBar.progressColor`).
    pub progress: Hsla,
    /// The colours of the indeterminate gradient
    /// (`ProgressBar.indeterminateStartColor`, `indeterminateEndColor`).
    pub indeterminate_start: Hsla,
    pub indeterminate_end: Hsla,
}

impl BarColors {
    pub fn from_theme(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            track: theme.color(UiRole::ProgressTrack),
            progress: theme.color(UiRole::Progress),
            indeterminate_start: theme.color(UiRole::ProgressIndeterminateStart),
            indeterminate_end: theme.color(UiRole::ProgressIndeterminateEnd),
        }
    }
}

/// Draws progress bars. A theme or a plugin may set its own with
/// [`set_painter`]; the bar's place and size stay the workbench's.
pub trait ProgressBarPainter: 'static {
    /// Height of the bar.
    fn height(&self) -> Pixels {
        px(HEIGHT)
    }

    /// Paints the bar in `bounds` (as wide as the bar and [`height`] high,
    /// centred in the space the bar was given).
    ///
    /// [`height`]: ProgressBarPainter::height
    fn paint(&self, bounds: Bounds<Pixels>, state: &BarState, colors: &BarColors, window: &mut Window);
}

/// The reference's progress bar (`DarculaProgressBarUI`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReferencePainter;

impl ProgressBarPainter for ReferencePainter {
    fn paint(&self, bounds: Bounds<Pixels>, state: &BarState, colors: &BarColors, window: &mut Window) {
        let h = bounds.size.height;
        let radius = h / 2.0;
        match state.fraction.filter(|f| *f > 0.0) {
            Some(fraction) => {
                window.paint_quad(fill(bounds, colors.track).corner_radii(radius));
                let done = done_width(f32::from(bounds.size.width), fraction);
                let done = Bounds::new(bounds.origin, size(px(done), h));
                window.paint_quad(fill(done, colors.progress).corner_radii(radius));
            }
            None => {
                let x0 = f32::from(bounds.origin.x);
                let width = f32::from(bounds.size.width);
                let segments = gradient_segments(width, state.frame);
                let last = segments.len().saturating_sub(1);
                for (i, s) in segments.iter().enumerate() {
                    let from = mix(colors.indeterminate_start, colors.indeterminate_end, s.t0);
                    let to = mix(colors.indeterminate_start, colors.indeterminate_end, s.t1);
                    let b = Bounds::new(
                        point(px(x0 + s.x0), bounds.origin.y),
                        size(px(s.x1 - s.x0), h),
                    );
                    let mut corners = gpui::Corners::all(px(0.0));
                    if i == 0 {
                        corners.top_left = radius;
                        corners.bottom_left = radius;
                    }
                    if i == last {
                        corners.top_right = radius;
                        corners.bottom_right = radius;
                    }
                    let background =
                        linear_gradient(90.0, linear_color_stop(from, 0.0), linear_color_stop(to, 1.0));
                    window.paint_quad(fill(b, background).corner_radii(corners));
                }
            }
        }
    }
}

/// Width of the done part: Swing's bar of 0 to 100 set to
/// `fraction * 99 + 1` (`ProgressComponent.updateProgressNow`).
pub fn done_width(width: f32, fraction: f32) -> f32 {
    let value = (fraction.clamp(0.0, 1.0) * 99.0 + 1.0).floor();
    (width * value / 100.0).round()
}

/// A piece of the indeterminate gradient: from `x0` to `x1` (relative to
/// the bar's left edge) the colour goes from `t0` to `t1` (0 the start
/// colour, 1 the end colour).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub x0: f32,
    pub x1: f32,
    pub t0: f32,
    pub t1: f32,
}

/// The reference's cyclic `GradientPaint` cut into linear pieces: the start
/// colour at `frame * 2 * STEP`, the end colour `FRAMES * STEP` further,
/// back to the start colour as far again, and so on in both directions.
pub fn gradient_segments(width: f32, frame: u32) -> Vec<Segment> {
    let half = FRAMES as f32 * STEP;
    let origin = (frame % FRAMES) as f32 * STEP * 2.0;
    let mut segments = Vec::new();
    if width <= 0.0 {
        return segments;
    }
    // Piece `k` runs from `origin + k * half`; even pieces go from the
    // start colour to the end one, odd ones back.
    let mut k = (-origin / half).floor() as i64;
    loop {
        let a = origin + k as f32 * half;
        if a >= width {
            break;
        }
        let x0 = a.max(0.0);
        let x1 = (a + half).min(width);
        let t = |x: f32| {
            let u = (x - a) / half;
            if k.rem_euclid(2) == 0 { u } else { 1.0 - u }
        };
        if x1 > x0 {
            segments.push(Segment {
                x0,
                x1,
                t0: t(x0),
                t1: t(x1),
            });
        }
        k += 1;
    }
    segments
}

/// The colour `t` of the way from `a` to `b`.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let l = |x: f32, y: f32| x + (y - x) * t;
    Hsla::from(Rgba {
        r: l(a.r, b.r),
        g: l(a.g, b.g),
        b: l(a.b, b.b),
        a: l(a.a, b.a),
    })
}

struct ActivePainter(Rc<dyn ProgressBarPainter>);

impl Global for ActivePainter {}

/// Draws every progress bar with `painter` from now on.
pub fn set_painter(cx: &mut App, painter: impl ProgressBarPainter) {
    cx.set_global(ActivePainter(Rc::new(painter)));
}

/// The painter in use.
pub fn painter(cx: &App) -> Rc<dyn ProgressBarPainter> {
    match cx.try_global::<ActivePainter>() {
        Some(p) => p.0.clone(),
        None => Rc::new(ReferencePainter),
    }
}

/// A progress bar filling the space it is given, its stripe centred
/// vertically.
pub fn progress_bar(state: BarState, cx: &App) -> impl IntoElement + use<> {
    let painter = painter(cx);
    let colors = BarColors::from_theme(cx);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let h = painter.height().min(bounds.size.height);
            let y = bounds.origin.y + (bounds.size.height - h) / 2.0;
            let stripe = Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, h));
            painter.paint(stripe, &state, &colors, window);
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_clock() {
        let start = Instant::now();
        assert_eq!(FRAMES, 16);
        assert_eq!(animation_frame(start, start), 0);
        assert_eq!(animation_frame(start, start + Duration::from_millis(149)), 2);
        assert_eq!(animation_frame(start, start + CYCLE), 0);
        assert_eq!(animation_frame(start, start + CYCLE + FRAME * 3), 3);
    }

    #[test]
    fn done_part_as_swing_draws_it() {
        assert_eq!(done_width(104.0, 0.0), 1.0);
        assert_eq!(done_width(104.0, 0.5), 52.0);
        assert_eq!(done_width(104.0, 1.0), 104.0);
    }

    fn colour_at(segments: &[Segment], x: f32) -> f32 {
        let s = segments
            .iter()
            .find(|s| s.x0 <= x && x <= s.x1)
            .expect("covered");
        s.t0 + (s.t1 - s.t0) * (x - s.x0) / (s.x1 - s.x0)
    }

    #[test]
    fn gradient_covers_the_bar_and_repeats() {
        // Frame 0: the start colour at 0, the end colour at 96, the start
        // colour again at 192.
        let s = gradient_segments(104.0, 0);
        assert_eq!(s.first().unwrap().x0, 0.0);
        assert_eq!(s.last().unwrap().x1, 104.0);
        for w in s.windows(2) {
            assert_eq!(w[0].x1, w[1].x0);
        }
        assert!((colour_at(&s, 0.0) - 0.0).abs() < 1e-4);
        assert!((colour_at(&s, 48.0) - 0.5).abs() < 1e-4);
        assert!((colour_at(&s, 96.0) - 1.0).abs() < 1e-4);
        assert!((colour_at(&s, 100.0) - (1.0 - 4.0 / 96.0)).abs() < 1e-4);
    }

    #[test]
    fn gradient_moves_twelve_pixels_a_frame() {
        let s = gradient_segments(300.0, 1);
        assert!((colour_at(&s, 12.0) - 0.0).abs() < 1e-4);
        assert!((colour_at(&s, 108.0) - 1.0).abs() < 1e-4);
        // Left of the origin the gradient comes from the end colour.
        assert!((colour_at(&s, 0.0) - 12.0 / 96.0).abs() < 1e-4);
        // A whole cycle later it is where it started.
        assert_eq!(gradient_segments(300.0, 16), gradient_segments(300.0, 0));
    }

    #[test]
    fn indeterminate_states() {
        let frame = 0;
        assert!(BarState { fraction: None, frame }.is_indeterminate());
        assert!(BarState { fraction: Some(0.0), frame }.is_indeterminate());
        assert!(!BarState { fraction: Some(0.1), frame }.is_indeterminate());
    }
}
