//! Popups at a position in an editor that leave the keyboard focus in the
//! editor (completion, and later quick documentation and parameter info),
//! as the reference's lookup and hints do: keys go to the editor, and while
//! a popup shows, the editor's container gets the popup's key context
//! (`Completion`), so the popup's bindings (`Completion > Editor`) win
//! over the editor's own.

use editor::Editor;
use gpui::{
    AnyElement, AnyView, App, Context, Corner, Entity, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, Styled, Window, anchored, deferred, div, point,
    px,
};
use themes::{Metric, UiRole};
use ui::ThemeExt;

/// Where a popup goes relative to the line of its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Under the line; above it when the window has no room below for the
    /// popup's height (when known).
    Below,
    /// Above the line (parameter info).
    Above,
}

/// A popup attached to a position in a tab's editor.
#[derive(Clone)]
pub struct CaretPopup {
    /// The key context added around the editor while the popup shows
    /// (`Completion`); it also names the popup (one popup per context).
    pub context: &'static str,
    pub editor: Entity<Editor>,
    /// The char offset the popup belongs to: it opens at its column, under
    /// or above its line.
    pub anchor: usize,
    pub placement: Placement,
    /// How far the popup's left edge is left of the anchor's (completion
    /// rows align their text with the typed prefix).
    pub inset: Pixels,
    /// The height of the popup when it is known in advance, to choose
    /// between below and above the line.
    pub height: Option<Pixels>,
    pub content: AnyView,
}

/// The caret popups of a window.
#[derive(Default)]
pub struct PopupHost {
    popups: Vec<CaretPopup>,
}

/// Room between a popup and the window's edges.
const MARGIN: f32 = 4.0;

impl PopupHost {
    /// Shows a popup, in place of the one with the same context.
    pub fn show(&mut self, popup: CaretPopup) {
        match self.popups.iter_mut().find(|p| p.context == popup.context) {
            Some(shown) => *shown = popup,
            None => self.popups.push(popup),
        }
    }

    /// Hides the popup of a context; false when none was shown.
    pub fn hide(&mut self, context: &str) -> bool {
        let before = self.popups.len();
        self.popups.retain(|p| p.context != context);
        self.popups.len() != before
    }

    /// Hides every popup of an editor.
    pub fn hide_editor(&mut self, editor: EntityId) {
        self.popups.retain(|p| p.editor.entity_id() != editor);
    }

    pub fn clear(&mut self) {
        self.popups.clear();
    }

    pub fn get(&self, context: &str) -> Option<&CaretPopup> {
        self.popups.iter().find(|p| p.context == context)
    }

    pub fn is_shown(&self, context: &str) -> bool {
        self.get(context).is_some()
    }

    /// The contexts of the popups shown, in the order they opened.
    pub fn contexts(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.popups.iter().map(|p| p.context)
    }

    /// The key context to add around an editor: the contexts of its popups
    /// (`"Completion ParameterInfo"`), `None` without popups.
    pub fn key_context(&self, editor: EntityId) -> Option<String> {
        let contexts: Vec<&str> = self
            .popups
            .iter()
            .filter(|p| p.editor.entity_id() == editor)
            .map(|p| p.context)
            .collect();
        (!contexts.is_empty()).then(|| contexts.join(" "))
    }

    /// The popups, positioned at their anchors in the editors' last frame
    /// and drawn above everything else; a popup whose anchor is not on the
    /// screen is left out.
    pub fn render(&self, window: &Window, cx: &App) -> Vec<AnyElement> {
        let viewport = window.viewport_size();
        self.popups
            .iter()
            .filter_map(|popup| {
                let editor = popup.editor.read(cx);
                let anchor = popup.anchor.min(editor.buffer().len_chars());
                let anchor_line = editor.buffer().char_to_line(anchor);
                if !editor.visible_lines()?.contains(&anchor_line) {
                    return None;
                }
                let line = editor.bounds_for_range(anchor..anchor)?;
                let x = line.left() - popup.inset;
                let below = match popup.placement {
                    Placement::Above => false,
                    Placement::Below => popup.height.is_none_or(|h| {
                        line.bottom() + h + px(MARGIN) <= viewport.height
                            || line.top() - h - px(MARGIN) < px(0.0)
                    }),
                };
                let (position, corner) = if below {
                    (point(x, line.bottom()), Corner::TopLeft)
                } else {
                    (point(x, line.top()), Corner::BottomLeft)
                };
                let context = popup.context;
                Some(
                    deferred(
                        anchored()
                            .position(position)
                            .anchor(corner)
                            .snap_to_window_with_margin(px(MARGIN))
                            .child(
                                div()
                                    .id(context)
                                    .debug_selector(move || format!("popup-{context}"))
                                    // Clicks stay in the popup: the editor
                                    // under it keeps its caret and focus.
                                    .occlude()
                                    .child(popup.content.clone()),
                            ),
                    )
                    .with_priority(1)
                    .into_any_element(),
                )
            })
            .collect()
    }
}

/// A one-line hint at a position (the reference's light hints, such as
/// "No suggestions" when Basic Completion finds nothing).
pub struct TextHint {
    text: SharedString,
}

impl TextHint {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }
}

impl Render for TextHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .debug_selector(|| "text-hint".into())
            .px(px(8.0))
            .py(px(4.0))
            .rounded(theme.metric(Metric::CornerRadius))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .whitespace_nowrap()
            .child(self.text.clone())
    }
}
