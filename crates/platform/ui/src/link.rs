//! Links, as the reference IDE draws them (`ActionLink`, `DropDownLink`,
//! `HyperlinkLabel`): text in the link colour, the pointing hand and, under
//! the mouse, the underline (`DefaultLinkButtonUI`). A link is made with
//! [`link`]; the caller adds the click handler and anything after the text
//! (a drop-down arrow).

use std::ops::Range;

use gpui::{
    App, CursorStyle, Div, ElementId, InteractiveText, ParentElement, SharedString, Stateful,
    Styled, StyledText, Window, div, prelude::*, px,
};
use themes::UiRole;

use crate::pointer;
use crate::theme::ThemeExt;

/// A link: the label in the link colour, underlined while the mouse is
/// over it, the pointing hand over it. GPUI lays text out before it knows
/// what is hovered, so a hover style cannot underline it; the underline
/// comes from the pointer module's state for the next frame (the window
/// redraws when it changes).
pub fn link(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let hovered = pointer::link_hovered(&id);
    div()
        .id(id.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .text_color(cx.theme().color(UiRole::TextLink))
        .child(div().when(hovered, |d| d.underline()).child(label.into()))
        .child(pointer::link_area(id))
}

/// Text with links in the `ranges`: a click in range `i` calls
/// `on_click(i)`, and the pointing hand shows over the ranges.
pub fn linked_text(
    id: impl Into<ElementId>,
    text: StyledText,
    ranges: Vec<Range<usize>>,
    on_click: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Div {
    let layout = text.layout().clone();
    let over = ranges.clone();
    div()
        .relative()
        .child(InteractiveText::new(id, text).on_click(ranges, on_click))
        .child(pointer::area_where(CursorStyle::PointingHand, move |p| {
            layout
                .index_for_position(p)
                .is_ok_and(|i| over.iter().any(|r| r.contains(&i)))
        }))
}
