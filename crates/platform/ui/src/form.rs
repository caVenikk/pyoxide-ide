//! Form widgets of dialogs and settings pages, as the reference lays them
//! out (the run configuration editor, the Kotlin UI DSL panels of its
//! settings pages, `DialogWrapper`'s south panel): a label column and rows
//! with their gaps, group headers (`TitledSeparator`), links, combo boxes
//! (the closed box; the list opens with [`MenuPopup`]), check box and radio
//! rows, the "?" help button and the bottom bar with the buttons in the
//! platform's order.
//!
//! The one-line text field is `editor::field` (an editor in one-line mode;
//! `ui` is below `editor`). Every colour and size comes from the theme.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Action, AnyElement, AnyWindowHandle, App, Context, Div, ElementId, Entity, FocusHandle,
    IntoElement, Pixels, Point, SharedString, Stateful, Subscription, Window, div, prelude::*, px,
};
use themes::{Metric, UiRole};

use crate::components::{DISABLED_ALPHA, Tooltip, checkbox};
use crate::icons::{Icon, IconName};
use crate::menu::{ChosenEvent, ContextMenu, MenuItem, PopupOptions};
use crate::theme::ThemeExt;

/// Height of a field (text field, combo box) and the space between rows.
pub const FIELD_HEIGHT: f32 = 28.0;
pub const ROW_GAP: f32 = 12.0;
/// Space between a check box or radio button and its label.
pub const CHECK_GAP: f32 = 6.0;
/// Size of a radio button.
pub const RADIO: f32 = 16.0;
/// The buttons of the bottom bar: narrowest width, height, the space
/// between two of them and the bar's height and side insets.
pub const BUTTON_WIDTH: f32 = 71.0;
pub const BUTTON_HEIGHT: f32 = 27.0;
pub const BUTTON_GAP: f32 = 13.0;
pub const BAR_HEIGHT: f32 = BUTTON_HEIGHT + 25.0;
pub const BAR_INSET: f32 = 15.0;
/// Size of the round help button.
pub const HELP_BUTTON: f32 = 22.0;
/// Space between a group header's title and its line
/// (`TitledSeparator.SEPARATOR_LEFT_INSET`).
const GROUP_LINE_GAP: f32 = 6.0;
/// The largest a list popup opens before it fits its rows.
const POPUP_MAX: (f32, f32) = (640.0, 900.0);

/// The corner radius of fields and buttons.
pub fn field_radius(cx: &App) -> Pixels {
    cx.theme().metric(Metric::CornerRadius) * 1.5
}

/// Rows of a form, one under the other with the row gap.
pub fn rows() -> Div {
    div().flex().flex_col().gap(px(ROW_GAP))
}

/// A row with a label column `width` wide (the label on one line); the
/// caller adds the field. A row without a label keeps the column empty.
pub fn label_row(width: f32, label: Option<impl IntoElement>) -> Div {
    div().flex().flex_row().items_center().child(
        div()
            .flex_none()
            .w(px(width))
            .whitespace_nowrap()
            .children(label),
    )
}

/// A comment under a field: muted text.
pub fn comment(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_color(cx.theme().color(UiRole::TextMuted))
        .child(text.into())
}

/// A group header (`TitledSeparator`): the title, then a line to the end
/// of the row.
pub fn group_header(title: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_row()
        .items_center()
        .child(div().flex_none().whitespace_nowrap().child(title.into()))
        .child(
            div()
                .flex_1()
                .ml(px(GROUP_LINE_GAP))
                .h(px(1.0))
                .bg(theme.color(UiRole::Divider)),
        )
}

/// The pointer is over an element. GPUI lays text out before it knows
/// what is hovered, so a hover style cannot underline text; the state keeps
/// it for the next frame (the window redraws when it changes).
#[derive(Clone, Default)]
pub struct Hover(Rc<Cell<bool>>);

impl Hover {
    pub fn get(&self) -> bool {
        self.0.get()
    }
}

/// A link (`ActionLink`, `DropDownLink`): text in the link colour, the
/// hand and the underline under the pointer. The caller adds the click
/// handler and anything after the text (a drop-down arrow).
pub fn link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    hover: &Hover,
    cx: &App,
) -> Stateful<Div> {
    let cell = hover.0.clone();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .text_color(cx.theme().color(UiRole::TextLink))
        .cursor_pointer()
        .on_hover(move |hovered: &bool, window, _| {
            if cell.get() != *hovered {
                cell.set(*hovered);
                window.refresh();
            }
        })
        .child(
            div()
                .when(hover.get(), |d| d.underline())
                .child(label.into()),
        )
}

/// A combo box, closed (`ComboBox` in the New UI's dialogs): filled, the
/// content on one line, the arrow at its end. The caller opens the list on
/// click ([`MenuPopup`]).
pub fn combo(id: impl Into<ElementId>, content: Vec<AnyElement>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(FIELD_HEIGHT))
        .px(px(8.0))
        .rounded(field_radius(cx))
        .border_1()
        .border_color(theme.color(UiRole::BorderInput))
        .bg(theme.color(UiRole::SurfaceRaised))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .children(content),
        )
        .child(div().flex_none().child(Icon::new(IconName::CHEVRON_DOWN)))
}

/// A small button inside a field (Insert Macros, Browse, Expand); without
/// an action it is drawn disabled.
pub fn field_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    action: Option<Box<dyn Action>>,
) -> AnyElement {
    let enabled = action.is_some();
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.0))
        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
        .child(Icon::new(icon))
        .tooltip(Tooltip::text(tooltip, None))
        .when_some(action, |d, action| {
            d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        })
        .into_any_element()
}

/// The Browse button of a path field (`TextFieldWithBrowseButton`).
pub fn browse_button(id: impl Into<ElementId>, action: Option<Box<dyn Action>>) -> AnyElement {
    field_button(id, IconName::BROWSE, "Browse", action)
}

/// A check box with its label; the caller adds the click handler on the
/// row (the check box and the label toggle together).
pub fn check_row(
    id: impl Into<ElementId>,
    check_id: impl Into<ElementId>,
    checked: bool,
    label: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(CHECK_GAP))
        .child(checkbox(check_id, checked, cx))
        .child(label)
}

/// A radio button: a circle with the input outline; selected, filled with
/// the accent colour and a dot.
pub fn radio(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(RADIO))
        .rounded_full()
        .when(selected, |d| {
            d.bg(theme.color(UiRole::Accent)).child(
                div()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(theme.color(UiRole::OnAccent)),
            )
        })
        .when(!selected, |d| {
            d.border_1().border_color(theme.color(UiRole::BorderInput))
        })
}

/// A radio button with its label; the caller adds the click handler.
pub fn radio_row(
    id: impl Into<ElementId>,
    radio_id: impl Into<ElementId>,
    selected: bool,
    label: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(CHECK_GAP))
        .child(radio(radio_id, selected, cx))
        .child(label)
}

/// The round "?" help button at the bottom left of a dialog; without an
/// action it is drawn disabled (the reference disables it for a dialog
/// without a help topic).
pub fn help_button(
    id: impl Into<ElementId>,
    action: Option<Box<dyn Action>>,
    cx: &App,
) -> AnyElement {
    let enabled = action.is_some();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(HELP_BUTTON))
        .rounded_full()
        .border_1()
        .border_color(cx.theme().color(UiRole::BorderInput))
        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
        .child(Icon::new(IconName::HELP))
        .tooltip(Tooltip::text("Help", None))
        .when_some(action, |d, action| {
            d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        })
        .into_any_element()
}

/// What a button of the bottom bar does, for its place on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    Ok,
    Cancel,
    Apply,
    Other,
}

/// A button of the bottom bar.
pub struct BarButton {
    id: SharedString,
    label: SharedString,
    role: ButtonRole,
    primary: bool,
    enabled: bool,
    action: Box<dyn Action>,
}

impl BarButton {
    /// A button running `action`; OK is the default button.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        role: ButtonRole,
        action: Box<dyn Action>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            role,
            primary: role == ButtonRole::Ok,
            enabled: true,
            action,
        }
    }

    /// The default button: filled with the accent colour, last on macOS.
    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Where it goes on macOS (`DialogWrapper.MAC_ACTION_ORDER`): Cancel
    /// before the others, the default button last.
    fn mac_order(&self) -> i32 {
        match (self.role, self.primary) {
            (_, true) => 100,
            (ButtonRole::Cancel, _) => -10,
            _ => 0,
        }
    }

    fn render(self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (accent, on_accent) = (theme.color(UiRole::Accent), theme.color(UiRole::OnAccent));
        let (input, hover) = (theme.color(UiRole::BorderInput), theme.color(UiRole::Hover));
        let disabled = theme.color(UiRole::TextDisabled);
        let Self {
            id,
            label,
            primary,
            enabled,
            action,
            ..
        } = self;
        let selector = id.clone();
        div()
            .id(ElementId::Name(id))
            .debug_selector(move || selector.to_string())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(BUTTON_HEIGHT))
            .min_w(px(BUTTON_WIDTH))
            .px(px(14.0))
            .rounded(field_radius(cx))
            .map(|d| {
                if primary {
                    d.bg(accent).text_color(on_accent)
                } else {
                    d.border_1()
                        .border_color(input)
                        .when(enabled, |d| d.hover(move |s| s.bg(hover)))
                }
            })
            .when(!enabled, |d| d.text_color(disabled))
            .child(label)
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
            .into_any_element()
    }
}

/// An item of the bottom bar: a button, or an element of the dialog's
/// own (the Run split button) with its place on macOS and the space after
/// it.
enum BarItem {
    Button(BarButton),
    Element {
        element: AnyElement,
        mac_order: i32,
        gap: f32,
    },
}

impl BarItem {
    fn mac_order(&self) -> i32 {
        match self {
            Self::Button(b) => b.mac_order(),
            Self::Element { mac_order, .. } => *mac_order,
        }
    }
}

/// The order of the bottom bar's buttons: on macOS sorted by their place
/// (Cancel, the others, the default button), elsewhere as given (the
/// reference's `createActions` order: OK, Cancel, Apply).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonOrder {
    Mac,
    AsGiven,
}

impl ButtonOrder {
    /// The order of this platform.
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::AsGiven
        }
    }
}

/// The bottom bar of a dialog (`DialogWrapper`'s south panel): the help
/// button, a status (the error), then the buttons on the right, separated
/// from the content by a line. Buttons are added in the reference's
/// `createActions` order (OK, Cancel, Apply); the bar puts them in the
/// platform's order.
pub struct BottomBar {
    help: Option<AnyElement>,
    status: Option<AnyElement>,
    items: Vec<BarItem>,
    order: ButtonOrder,
}

impl Default for BottomBar {
    fn default() -> Self {
        Self::new()
    }
}

impl BottomBar {
    pub fn new() -> Self {
        Self {
            help: None,
            status: None,
            items: Vec::new(),
            order: ButtonOrder::native(),
        }
    }

    /// The help button at the left ([`help_button`]).
    pub fn help(mut self, help: AnyElement) -> Self {
        self.help = Some(help);
        self
    }

    /// Text between the help button and the buttons, on one line.
    pub fn status(mut self, status: impl IntoElement) -> Self {
        self.status = Some(status.into_any_element());
        self
    }

    pub fn button(mut self, button: BarButton) -> Self {
        self.items.push(BarItem::Button(button));
        self
    }

    /// An element of the dialog's own among the buttons, `gap` before the
    /// next one; `mac_order` places it on macOS (Cancel is -10, the
    /// default button 100, others 0).
    pub fn element(mut self, element: impl IntoElement, mac_order: i32, gap: f32) -> Self {
        self.items.push(BarItem::Element {
            element: element.into_any_element(),
            mac_order,
            gap,
        });
        self
    }

    /// Overrides the platform's order (tests).
    pub fn order(mut self, order: ButtonOrder) -> Self {
        self.order = order;
        self
    }

    /// The ids of the buttons in the order they are drawn (tests).
    pub fn button_ids(&self) -> Vec<SharedString> {
        self.ordered_indices()
            .into_iter()
            .filter_map(|i| match &self.items[i] {
                BarItem::Button(b) => Some(b.id.clone()),
                BarItem::Element { .. } => None,
            })
            .collect()
    }

    fn ordered_indices(&self) -> Vec<usize> {
        let mut at: Vec<usize> = (0..self.items.len()).collect();
        if self.order == ButtonOrder::Mac {
            at.sort_by_key(|&i| self.items[i].mac_order());
        }
        at
    }

    pub fn render(self, cx: &App) -> AnyElement {
        let order = self.ordered_indices();
        let mut items: Vec<Option<BarItem>> = self.items.into_iter().map(Some).collect();
        let count = order.len();
        let mut buttons: Vec<AnyElement> = Vec::new();
        for (n, i) in order.into_iter().enumerate() {
            let Some(item) = items[i].take() else {
                continue;
            };
            let gap = match item {
                BarItem::Button(b) => {
                    buttons.push(b.render(cx));
                    BUTTON_GAP
                }
                BarItem::Element { element, gap, .. } => {
                    buttons.push(element);
                    gap
                }
            };
            if n + 1 < count {
                buttons.push(div().w(px(gap)).into_any_element());
            }
        }
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(BAR_HEIGHT))
            .pl(px(BAR_INSET))
            .pr(px(BAR_INSET))
            .border_t_1()
            .border_color(cx.theme().color(UiRole::Border))
            .children(self.help)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(px(12.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .children(self.status),
            )
            .children(buttons)
            .into_any_element()
    }
}

/// A list popup of a dialog (a combo box's values, a drop-down link's
/// menu): a `ContextMenu` in a window of its own
/// ([`crate::dialog_window::open_popup`]), so that it may reach beyond the
/// dialog. The chosen row's action runs in the dialog's window.
pub struct MenuPopup {
    pub menu: Entity<ContextMenu>,
    pub window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl MenuPopup {
    /// Opens `items` at `position` (in `window`'s coordinates) with speed
    /// search; the shortcuts shown are the ones of `focus`. `on_close` runs
    /// when the popup is dismissed (closing it is the owner's: [`Self::close`]).
    #[allow(clippy::too_many_arguments)] // the parts of a popup the owner chooses
    pub fn open<T: 'static>(
        items: Vec<MenuItem>,
        title: Option<SharedString>,
        position: Point<Pixels>,
        focus: &FocusHandle,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<T>,
        on_close: impl Fn(&mut T, &mut Context<T>) + 'static,
    ) -> Option<Self> {
        let mut items = items;
        crate::menu::fill_shortcuts(&mut items, focus, window);
        let options = PopupOptions {
            title,
            speed_search: true,
            own_window: true,
            ..PopupOptions::default()
        };
        let mut built = None;
        let popup = crate::dialog_window::open_popup(
            window,
            position,
            gpui::size(px(POPUP_MAX.0), px(POPUP_MAX.1)),
            activate,
            cx,
            |window, cx| {
                let menu = ContextMenu::popup(items, options, window, cx);
                built = Some(menu.clone());
                menu.into()
            },
        );
        let (popup, menu) = (popup?, built?);
        let subscriptions = vec![
            cx.subscribe_in(&menu, window, |_, _, chosen: &ChosenEvent, window, cx| {
                window.dispatch_action(chosen.0.boxed_clone(), cx);
            }),
            cx.subscribe(&menu, move |this, _, _: &gpui::DismissEvent, cx| {
                on_close(this, cx)
            }),
        ];
        Some(Self {
            menu,
            window: popup.into(),
            _subscriptions: subscriptions,
        })
    }

    /// Closes the popup's window.
    pub fn close(self, cx: &mut App) {
        crate::dialog_window::close(self.window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    gpui::actions!(form_test, [Accept, Reject, Store]);

    fn bar(order: ButtonOrder) -> BottomBar {
        BottomBar::new()
            .order(order)
            .element(div(), -100, 19.0)
            .button(BarButton::new("ok", "OK", ButtonRole::Ok, Box::new(Accept)))
            .button(BarButton::new(
                "cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(Reject),
            ))
            .button(BarButton::new(
                "apply",
                "Apply",
                ButtonRole::Apply,
                Box::new(Store),
            ))
    }

    /// macOS: `Cancel | Apply | OK`; Windows and Linux: `OK | Cancel |
    /// Apply` (`DialogWrapper.createSouthPanel`).
    #[test]
    fn the_buttons_take_the_platforms_order() {
        assert_eq!(
            bar(ButtonOrder::Mac).button_ids(),
            ["cancel", "apply", "ok"]
        );
        assert_eq!(
            bar(ButtonOrder::AsGiven).button_ids(),
            ["ok", "cancel", "apply"]
        );
        assert_eq!(
            BottomBar::new().order,
            if cfg!(target_os = "macos") {
                ButtonOrder::Mac
            } else {
                ButtonOrder::AsGiven
            }
        );
    }
}
