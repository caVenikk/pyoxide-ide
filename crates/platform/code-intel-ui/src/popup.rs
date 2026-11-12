//! Popups at a position in an editor that leave the keyboard focus in the
//! editor (completion, and later quick documentation and parameter info),
//! as the reference's lookup and hints do: keys go to the editor, and while
//! a popup shows, the editor's container gets the popup's key context
//! (`Completion`), so the popup's bindings (`Completion > Editor`) win
//! over the editor's own.

use editor::Editor;
use gpui::{
    AnyElement, AnyView, App, Context, Corner, Entity, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, Styled, Window, anchored, canvas, div, point, px,
};
use themes::{Metric, UiRole};
use ui::ThemeExt;
use ui::deferred;

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
    pub fn show(&mut self, popup: CaretPopup, cx: &mut App) {
        match self.popups.iter_mut().find(|p| p.context == popup.context) {
            Some(shown) => {
                if shown.content.entity_id() != popup.content.entity_id() {
                    ui::popup_window::close(shown.content.entity_id(), cx);
                }
                *shown = popup;
            }
            None => self.popups.push(popup),
        }
    }

    /// Hides the popup of a context; false when none was shown.
    pub fn hide(&mut self, context: &str, cx: &mut App) -> bool {
        let before = self.popups.len();
        self.popups.retain(|p| {
            if p.context == context {
                ui::popup_window::close(p.content.entity_id(), cx);
                false
            } else {
                true
            }
        });
        self.popups.len() != before
    }

    /// Hides every popup of an editor.
    pub fn hide_editor(&mut self, editor: EntityId, cx: &mut App) {
        self.popups.retain(|p| {
            if p.editor.entity_id() == editor {
                ui::popup_window::close(p.content.entity_id(), cx);
                false
            } else {
                true
            }
        });
    }

    pub fn clear(&mut self, cx: &mut App) {
        for popup in &self.popups {
            ui::popup_window::close(popup.content.entity_id(), cx);
        }
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
                if !editor
                    .visible_lines()
                    .is_some_and(|lines| lines.contains(&anchor_line))
                {
                    let id = popup.content.entity_id();
                    return Some(
                        canvas(
                            move |_, _, cx| ui::popup_window::close(id, cx),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_0()
                        .into_any_element(),
                    );
                }
                let line = editor.bounds_for_range(anchor..anchor)?;
                let x = line.left() - popup.inset;
                let room_below = viewport.height - line.bottom() - px(MARGIN);
                let room_above = line.top() - px(MARGIN);
                let below = below(popup.placement, popup.height, room_below, room_above);
                // A popup higher than the room on its side gets that room;
                // its content scrolls (popups whose content can shrink).
                let room = if below { room_below } else { room_above };
                let (position, corner) = if below {
                    (point(x, line.bottom()), Corner::TopLeft)
                } else {
                    (point(x, line.top()), Corner::BottomLeft)
                };
                let context = popup.context;
                let interactive_doc = context == crate::doc::CONTEXT
                    && popup
                        .content
                        .clone()
                        .downcast::<crate::doc::DocPopup>()
                        .ok()
                        .and_then(|doc| {
                            doc.read(cx).content().map(|content| {
                                content.documentation.is_some()
                                    || content.fix.is_some()
                                    || content.more_actions
                            })
                        })
                        .unwrap_or(false);
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
                                    .flex()
                                    .flex_col()
                                    .max_h(room.max(px(0.0)))
                                    .child(
                                        if ui::popup_window::enabled()
                                            && (context == "Completion" || interactive_doc)
                                        {
                                            ui::popup_window::attached_view(
                                                &popup.editor,
                                                popup.content.clone(),
                                                room.max(px(1.0)),
                                            )
                                        } else if ui::popup_window::enabled()
                                            && context == "CodeList"
                                        {
                                            let list = popup
                                                .content
                                                .clone()
                                                .downcast::<crate::list_popup::ListPopup>()
                                                .expect("CodeList content");
                                            let outside = list.clone();
                                            div()
                                                .on_mouse_down_out(move |_, window, cx| {
                                                    outside.update(cx, |list, cx| {
                                                        list.dismiss_outside(window, cx)
                                                    });
                                                })
                                                .child(ui::popup_window::view_with_dismiss(
                                                    &list,
                                                    crate::list_popup::ListPopup::dismiss_outside,
                                                ))
                                                .into_any_element()
                                        } else {
                                            popup.content.clone().into_any_element()
                                        },
                                    ),
                            ),
                    )
                    .with_priority(1)
                    .into_any_element(),
                )
            })
            .collect()
    }
}

/// Whether a popup goes under its line: on its preferred side when its
/// height (when known) fits there, else on the other side when it fits
/// there, else on the side with more room.
pub fn below(placement: Placement, height: Option<Pixels>, below: Pixels, above: Pixels) -> bool {
    let fits = |room: Pixels| height.is_none_or(|h| h <= room);
    match placement {
        Placement::Below => fits(below) || (!fits(above) && below >= above),
        Placement::Above => !fits(above) && (fits(below) || below > above),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popups_go_where_they_fit() {
        let (h, room) = (Some(px(100.0)), |v: f32| px(v));
        // Room on the preferred side.
        assert!(below(Placement::Below, h, room(150.0), room(50.0)));
        assert!(!below(Placement::Above, h, room(150.0), room(120.0)));
        // Not there, but on the other side.
        assert!(!below(Placement::Below, h, room(50.0), room(150.0)));
        assert!(below(Placement::Above, h, room(150.0), room(50.0)));
        // Nowhere: the side with more room.
        assert!(below(Placement::Below, h, room(80.0), room(60.0)));
        assert!(!below(Placement::Below, h, room(60.0), room(80.0)));
        assert!(below(Placement::Above, h, room(80.0), room(60.0)));
        assert!(!below(Placement::Above, h, room(60.0), room(80.0)));
        // Height not known: the preferred side.
        assert!(below(Placement::Below, None, room(0.0), room(500.0)));
        assert!(!below(Placement::Above, None, room(500.0), room(0.0)));
    }
}
