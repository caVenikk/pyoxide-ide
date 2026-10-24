//! Adjust Font Size of the documentation popup: a small popup at the
//! mouse with "Font size:" and a slider over the sizes of documentation
//! text (`FontSizePopup.showFontSizePopup`: a `JBSlider` with a tick per
//! size that snaps to them, in a panel with a line border, centred on the
//! pointer). Moving the slider stores the size at once and the
//! documentation follows; a click outside or Escape closes it.
//!
//! The slider is drawn as the reference's `DarculaSliderUI`: a track 3
//! thick, a pentagon thumb 15 wide pointing down, ticks 4 high under the
//! track.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, PathBuilder, Pixels, Point, Render, Styled, Window, canvas, div, point,
    prelude::FluentBuilder, px, rgb,
};
use serde_json::{Value as Json, json};
use settings::{DocFontSize, DocumentationSettings};
use themes::UiRole;
use ui::ThemeExt;

/// The label before the slider (`ApplicationBundle`'s `label.font.size`).
pub const LABEL: &str = "Font size:";

// `JSlider`'s preferred width and the height of a horizontal slider with
// ticks (thumb 24 and tick length 7), the gaps of the panel's
// `FlowLayout(RIGHT, 3, 0)`.
const SLIDER_WIDTH: f32 = 200.0;
const SLIDER_HEIGHT: f32 = 31.0;
const GAP: f32 = 3.0;
// `DarculaSliderUIThemes`, from the top left of the thumb's box.
const THUMB_HALF: f32 = 7.0;
const FOCUS_BORDER: f32 = 3.0;
/// Room left and right of the track (`trackBuffer`).
const TRACK_BUFFER: f32 = THUMB_HALF + FOCUS_BORDER + 1.0;
const THUMB_OVERHANG: f32 = 10.0;
const THUMB_HEIGHT: f32 = 24.0;
const TRACK_THICKNESS: f32 = 3.0;
const TICK_TOP: f32 = 6.0;
const MINOR_TICK: f32 = 4.0;

/// Asks to change the size; the documentation popup stores it.
pub struct SizeChosen(pub DocFontSize);

/// The open Adjust Font Size popup.
pub struct FontSizePopup {
    focus: FocusHandle,
    /// Had the focus before; gets it back when the popup closes.
    origin: Option<FocusHandle>,
    value: DocFontSize,
    /// The slider's bounds in the window, as last drawn.
    slider: Rc<Cell<Bounds<Pixels>>>,
    dismissed: bool,
}

impl EventEmitter<DismissEvent> for FontSizePopup {}
impl EventEmitter<SizeChosen> for FontSizePopup {}

impl Focusable for FontSizePopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FontSizePopup {
    /// Opens the popup with the stored size and gives it the focus.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let origin = window.focused(cx);
        window.focus(&focus);
        Self {
            focus,
            origin,
            value: settings_ui::store::get::<DocumentationSettings>(cx).font_size,
            slider: Rc::new(Cell::new(Bounds::default())),
            dismissed: false,
        }
    }

    pub fn value(&self) -> DocFontSize {
        self.value
    }

    /// Its size in points: the label, the slider and the gaps, in a line
    /// border.
    pub fn size(window: &Window, cx: &App) -> gpui::Size<Pixels> {
        let theme = cx.theme();
        let run = gpui::TextRun {
            len: LABEL.len(),
            font: gpui::font(theme.ui_family()),
            color: theme.color(UiRole::Text),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let label = window
            .text_system()
            .shape_line(LABEL.into(), theme.ui_size(), &[run], None)
            .width;
        gpui::size(
            label + px(SLIDER_WIDTH + 3.0 * GAP + 2.0),
            px(SLIDER_HEIGHT + 2.0),
        )
    }

    /// Sets the size (the slider moved); the documentation follows.
    pub fn choose(&mut self, value: DocFontSize, cx: &mut Context<Self>) {
        if value == self.value {
            return;
        }
        self.value = value;
        cx.emit(SizeChosen(value));
        cx.notify();
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let index = self.value.index() as isize + by;
        let last = DocFontSize::ALL.len() as isize - 1;
        self.choose(DocFontSize::at(index.clamp(0, last) as usize), cx);
    }

    /// The size whose tick is nearest to a point of the window.
    fn value_at(&self, at: Point<Pixels>) -> DocFontSize {
        let bounds = self.slider.get();
        let width = f32::from(bounds.size.width) - 2.0 * TRACK_BUFFER;
        let x = f32::from(at.x - bounds.origin.x) - TRACK_BUFFER;
        let steps = (DocFontSize::ALL.len() - 1) as f32;
        let index = (x / width * steps).round().clamp(0.0, steps);
        DocFontSize::at(index as usize)
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissed {
            return;
        }
        self.dismissed = true;
        if let Some(origin) = self.origin.take() {
            window.focus(&origin);
        }
        cx.emit(DismissEvent);
    }

    pub fn state_json(&self) -> Json {
        json!({ "label": LABEL, "value": self.value, "index": self.value.index() })
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let last = DocFontSize::ALL.len() - 1;
        match event.keystroke.key.as_str() {
            "left" | "down" => self.step(-1, cx),
            "right" | "up" => self.step(1, cx),
            "home" => self.choose(DocFontSize::at(0), cx),
            "end" => self.choose(DocFontSize::at(last), cx),
            "escape" | "enter" => self.dismiss(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// The colours of `DarculaSliderUIThemes` (dark and light defaults).
struct SliderColors {
    button: Hsla,
    button_border: Hsla,
    track: Hsla,
    tick: Hsla,
}

impl SliderColors {
    fn new(dark: bool) -> Self {
        let c = |light: u32, dark_value: u32| -> Hsla {
            rgb(if dark { dark_value } else { light }).into()
        };
        Self {
            button: c(0xFFFFFF, 0x9B9E9E),
            button_border: c(0xA6A6A6, 0x393D3F),
            track: c(0xC7C7C7, 0x666666),
            tick: c(0x999999, 0x808080),
        }
    }
}

/// Paints the slider in `bounds` with the thumb at `index` of `count`.
fn paint_slider(
    bounds: Bounds<Pixels>,
    index: usize,
    count: usize,
    colors: &SliderColors,
    window: &mut Window,
) {
    let o = bounds.origin;
    let track_x = o.x + px(TRACK_BUFFER);
    let track_w = bounds.size.width - px(2.0 * TRACK_BUFFER);
    let x_of = |i: usize| track_x + track_w * (i as f32 / (count - 1).max(1) as f32);
    // The track under the thumb's shoulders.
    let track_y = o.y + px(FOCUS_BORDER + THUMB_OVERHANG - TRACK_THICKNESS);
    window.paint_quad(gpui::fill(
        Bounds::new(
            point(track_x, track_y),
            gpui::size(track_w, px(TRACK_THICKNESS)),
        ),
        colors.track,
    ));
    // A minor tick per size.
    let tick_y = o.y + px(FOCUS_BORDER + THUMB_OVERHANG + TICK_TOP);
    for i in 0..count {
        window.paint_quad(gpui::fill(
            Bounds::new(point(x_of(i), tick_y), gpui::size(px(1.0), px(MINOR_TICK))),
            colors.tick,
        ));
    }
    // The thumb: a pentagon pointing down, centred on its tick.
    let left = x_of(index) - px(THUMB_HALF + 0.5);
    let x1 = left;
    let x2 = left + px(2.0 * THUMB_HALF + 1.0);
    let x3 = left + px(THUMB_HALF + 0.5);
    let y1 = o.y + px(FOCUS_BORDER);
    let y2 = y1 + px(THUMB_OVERHANG);
    let y3 = o.y + px(THUMB_HEIGHT - FOCUS_BORDER);
    let arc = px(1.0);
    let outline = [
        point(x1 + arc, y1),
        point(x2 - arc, y1),
        point(x2, y1 + arc),
        point(x2, y2),
        point(x3, y3),
        point(x1, y2),
        point(x1, y1 + arc),
    ];
    let mut fill = PathBuilder::fill();
    let mut stroke = PathBuilder::stroke(px(1.0));
    for (i, p) in outline.iter().enumerate() {
        if i == 0 {
            fill.move_to(*p);
            stroke.move_to(*p);
        } else {
            fill.line_to(*p);
            stroke.line_to(*p);
        }
    }
    fill.close();
    stroke.close();
    if let Ok(path) = fill.build() {
        window.paint_path(path, colors.button);
    }
    if let Ok(path) = stroke.build() {
        window.paint_path(path, colors.button_border);
    }
}

impl Render for FontSizePopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = SliderColors::new(theme.is_dark());
        let index = self.value.index();
        let count = DocFontSize::ALL.len();
        let measured = self.slider.clone();
        let slider = div()
            .id("doc-font-size-slider")
            .debug_selector(|| "doc-font-size-slider".into())
            .relative()
            .flex_none()
            .w(px(SLIDER_WIDTH))
            .h(px(SLIDER_HEIGHT))
            .child(
                canvas(
                    move |bounds, _, _| measured.set(bounds),
                    move |bounds, _, window, _| paint_slider(bounds, index, count, &colors, window),
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let value = this.value_at(e.position);
                    this.choose(value, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if e.pressed_button == Some(MouseButton::Left) {
                    let value = this.value_at(e.position);
                    this.choose(value, cx);
                }
            }));
        div()
            .id("doc-font-size")
            .debug_selector(|| "doc-font-size".into())
            .track_focus(&self.focus)
            .occlude()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GAP))
            .px(px(GAP))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .when(self.dismissed, |d| d.invisible())
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.dismiss(window, cx)),
            )
            .child(div().flex_none().whitespace_nowrap().child(LABEL))
            .child(slider)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    #[gpui::test]
    fn keys_step_through_the_sizes(cx: &mut TestAppContext) {
        cx.update(themes_init);
        let (popup, cx) = cx.add_window_view(|window, cx| FontSizePopup::new(window, cx));
        let chosen = Rc::new(Cell::new(None));
        let sink = chosen.clone();
        cx.update(|_, cx| {
            cx.subscribe(&popup, move |_, e: &SizeChosen, _| sink.set(Some(e.0)))
                .detach()
        });
        cx.simulate_keystrokes("right");
        assert_eq!(chosen.get(), Some(DocFontSize::Medium));
        cx.simulate_keystrokes("end");
        assert_eq!(chosen.get(), Some(DocFontSize::XxLarge));
        cx.simulate_keystrokes("right");
        assert_eq!(popup.read_with(cx, |p, _| p.value()), DocFontSize::XxLarge);
        cx.simulate_keystrokes("home left");
        assert_eq!(chosen.get(), Some(DocFontSize::XxSmall));
        dismiss_with_escape(&popup, cx);
    }

    fn dismiss_with_escape(popup: &gpui::Entity<FontSizePopup>, cx: &mut VisualTestContext) {
        let dismissed = Rc::new(Cell::new(false));
        let sink = dismissed.clone();
        cx.update(|_, cx| {
            cx.subscribe(popup, move |_, _: &DismissEvent, _| sink.set(true))
                .detach()
        });
        cx.simulate_keystrokes("escape");
        assert!(dismissed.get());
    }

    fn themes_init(cx: &mut App) {
        ui::init(cx, themes::ThemeKind::Dark);
    }
}
