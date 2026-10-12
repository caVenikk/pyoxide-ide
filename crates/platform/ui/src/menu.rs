//! Context menus, as the reference IDE's New UI draws them: a rounded popup
//! with a border and a shadow, rows with an optional icon column, rounded
//! selection, separators and submenus that open to the right of their item.
//!
//! A component builds the menu with [`ContextMenu::build`] where the person
//! right-clicked (or from a command), keeps the entity while it is open and
//! draws it with [`overlay`]. The menu takes the focus and gives it back to
//! the element that had it when it closes; the chosen item's action is then
//! dispatched there, exactly like a key binding, so menu items are UI
//! commands. The menu emits [`DismissEvent`] when it closes.
//!
//! Keyboard: Up and Down move through the innermost open level, Right and
//! Enter open a submenu, Left and Escape close it, Escape on the menu
//! itself closes the menu, Enter runs the selected item. A click outside
//! the menu closes it; so does deactivating the window.

use gpui::{
    Action, AnyElement, App, AppContext as _, BoxShadow, Context, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, anchored, deferred, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};

use crate::icons::{Icon, IconName};
use crate::theme::ThemeExt;

crate::ui_commands! {
    COMMANDS, scope = "ContextMenu";
    SelectNext => "menu.down", "Next Item",
        "Selects the next item of the innermost open menu level.";
    SelectPrevious => "menu.up", "Previous Item",
        "Selects the previous item of the innermost open menu level.";
    OpenSubmenu => "menu.open-submenu", "Open Submenu",
        "Opens the submenu of the selected item and selects its first item.";
    CloseSubmenu => "menu.close-submenu", "Close Submenu",
        "Closes the innermost open submenu.";
    Confirm => "menu.confirm", "Choose Item",
        "Runs the selected item, or opens its submenu.";
    Cancel => "menu.cancel", "Close",
        "Closes the innermost open submenu, or the menu.";
}

/// Key context of an open context menu.
pub const KEY_CONTEXT: &str = "ContextMenu";

// Metrics of the reference IDE's popup menus in the New UI (see NOTICE).
// An item row is a list row (`List.rowHeight`, the theme's row height)
// plus the selection's outer insets above and below (`IdeaMenuUI.
// patchPreferredSize`).

/// Selection background inside an item row: `PopupMenu.Selection.
/// outerInsets` (1, 7).
const SELECTION_X: f32 = 7.0;
const SELECTION_Y: f32 = 1.0;
/// Content inside the selection background: `PopupMenu.Selection.
/// innerInsets` (0, 6) of the New UI.
const CONTENT_X: f32 = 6.0;
/// Radius of the selection background: `PopupMenu.Selection.arc` 8 is a
/// diameter.
const SELECTION_RADIUS: f32 = 4.0;
/// Separator row: `PopupMenuSeparator.height` 9, a line of
/// `stripeWidth` 1 at `stripeIndent` 4, `withToEdge` 10 from both sides.
const SEPARATOR_HEIGHT: f32 = 9.0;
const SEPARATOR_INDENT: f32 = 4.0;
const SEPARATOR_X: f32 = 10.0;
/// Space above the first and below the last row: `PopupMenu.borderInsets`
/// (6, 1, 6, 1); the 1 is the border.
const PAD_Y: f32 = 6.0;
/// Popup corners: `PopupMenu.borderCornerRadius` 8.
const RADIUS: f32 = 8.0;
/// Icon column (16 px icons) and the gap to the label: Swing's menu item
/// `defaultTextIconGap` 4.
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 4.0;
/// Room after the label (`BegMenuItemUI.getPreferredSize`: the gutter of
/// 18 and four text gaps); a submenu's arrow sits at its right end.
const TRAILING: f32 = 18.0 + 4.0 * ICON_GAP;
/// Popup shadow: `Ide.Shadow` of the Islands themes, 16 px of `#00000020`
/// in both the dark and the light theme.
const SHADOW_BLUR: f32 = 16.0;
const SHADOW_ALPHA: f32 = 32.0 / 255.0;

/// One row of a menu.
pub enum MenuItem {
    /// Runs `action` where the focus was before the menu opened.
    Action {
        label: SharedString,
        icon: Option<IconName>,
        action: Box<dyn Action>,
        enabled: bool,
    },
    /// Opens `items` next to it.
    Submenu {
        label: SharedString,
        icon: Option<IconName>,
        items: Vec<MenuItem>,
    },
    Separator,
}

impl Clone for MenuItem {
    fn clone(&self) -> Self {
        match self {
            Self::Action {
                label,
                icon,
                action,
                enabled,
            } => Self::Action {
                label: label.clone(),
                icon: *icon,
                action: action.boxed_clone(),
                enabled: *enabled,
            },
            Self::Submenu { label, icon, items } => Self::Submenu {
                label: label.clone(),
                icon: *icon,
                items: items.clone(),
            },
            Self::Separator => Self::Separator,
        }
    }
}

impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action { label, action, .. } => {
                write!(f, "Action({label:?}, {})", action.name())
            }
            Self::Submenu { label, items, .. } => write!(f, "Submenu({label:?}, {items:?})"),
            Self::Separator => f.write_str("Separator"),
        }
    }
}

impl MenuItem {
    pub fn action(label: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Action {
            label: label.into(),
            icon: None,
            action: Box::new(action),
            enabled: true,
        }
    }

    pub fn submenu(label: impl Into<SharedString>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            icon: None,
            items,
        }
    }

    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn icon(mut self, name: IconName) -> Self {
        match &mut self {
            Self::Action { icon, .. } | Self::Submenu { icon, .. } => *icon = Some(name),
            Self::Separator => {}
        }
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        if let Self::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    pub fn label(&self) -> Option<&SharedString> {
        match self {
            Self::Action { label, .. } | Self::Submenu { label, .. } => Some(label),
            Self::Separator => None,
        }
    }

    fn icon_name(&self) -> Option<IconName> {
        match self {
            Self::Action { icon, .. } | Self::Submenu { icon, .. } => *icon,
            Self::Separator => None,
        }
    }

    /// Can be selected: not a separator, not disabled.
    fn selectable(&self) -> bool {
        match self {
            Self::Action { enabled, .. } => *enabled,
            Self::Submenu { items, .. } => !items.is_empty(),
            Self::Separator => false,
        }
    }

    fn height(&self, item_height: f32) -> f32 {
        match self {
            Self::Separator => SEPARATOR_HEIGHT,
            _ => item_height,
        }
    }
}

/// An open context menu.
pub struct ContextMenu {
    focus: FocusHandle,
    items: Vec<MenuItem>,
    /// The selected row of each open level: the menu itself first, then
    /// the submenu of its selected row, and so on.
    selected: Vec<Option<usize>>,
    /// Had the focus before; gets it back and runs the chosen action.
    origin: Option<FocusHandle>,
    dismissed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ContextMenu {
    /// Opens a menu of `items` and gives it the focus; the element that had
    /// the focus gets it back when the menu closes and receives the chosen
    /// action.
    pub fn build(items: Vec<MenuItem>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let origin = window.focused(cx);
        let menu = cx.new(|cx: &mut Context<Self>| {
            let focus = cx.focus_handle();
            let subscriptions = vec![
                cx.on_blur(&focus, window, |this, window, cx| this.dismiss(window, cx)),
                cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() {
                        this.dismiss(window, cx);
                    }
                }),
            ];
            Self {
                focus,
                items,
                selected: vec![None],
                origin,
                dismissed: false,
                _subscriptions: subscriptions,
            }
        });
        let handle = menu.read(cx).focus.clone();
        window.focus(&handle);
        menu
    }

    /// Closes the menu and gives the focus back.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissed {
            return;
        }
        self.dismissed = true;
        if self.focus.contains_focused(window, cx)
            && let Some(origin) = &self.origin
        {
            window.focus(origin);
        }
        cx.emit(DismissEvent);
        cx.notify();
    }

    fn level_items(&self, level: usize) -> &[MenuItem] {
        let mut items = self.items.as_slice();
        for selected in &self.selected[..level] {
            match selected.and_then(|i| items.get(i)) {
                Some(MenuItem::Submenu { items: sub, .. }) => items = sub,
                _ => return &[],
            }
        }
        items
    }

    fn innermost(&self) -> usize {
        self.selected.len() - 1
    }

    /// Selects row `ix` of `level`, closing deeper levels; a submenu row
    /// opens its submenu (with nothing selected yet).
    fn select(&mut self, level: usize, ix: usize, cx: &mut Context<Self>) {
        let Some(item) = self.level_items(level).get(ix) else {
            return;
        };
        if !item.selectable() {
            return;
        }
        let submenu = matches!(item, MenuItem::Submenu { .. });
        self.selected.truncate(level + 1);
        self.selected[level] = Some(ix);
        if submenu {
            self.selected.push(None);
        }
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let level = self.innermost();
        let items = self.level_items(level);
        let n = items.len() as isize;
        if n == 0 {
            return;
        }
        let start = self.selected[level].map_or(if delta > 0 { -1 } else { n }, |i| i as isize);
        let mut i = start;
        for _ in 0..n {
            i = (i + delta).rem_euclid(n);
            if items[i as usize].selectable() {
                self.selected[level] = Some(i as usize);
                cx.notify();
                return;
            }
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    /// Opens the submenu of the selected row of the innermost level and
    /// selects its first row; returns false if that row has no submenu.
    fn open_selected_submenu(&mut self, cx: &mut Context<Self>) -> bool {
        let level = self.innermost();
        let Some(ix) = self.selected[level] else {
            return false;
        };
        if !matches!(
            self.level_items(level).get(ix),
            Some(MenuItem::Submenu { .. })
        ) {
            return false;
        }
        self.select(level, ix, cx);
        self.step(1, cx);
        true
    }

    fn open_submenu(&mut self, _: &OpenSubmenu, _: &mut Window, cx: &mut Context<Self>) {
        // An open submenu with nothing selected (opened by the mouse) gets
        // its first row selected.
        let level = self.innermost();
        if level > 0 && self.selected[level].is_none() {
            self.step(1, cx);
            return;
        }
        self.open_selected_submenu(cx);
    }

    fn close_submenu(&mut self, _: &CloseSubmenu, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.len() > 1 {
            self.selected.pop();
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.len() > 1 {
            self.selected.pop();
            cx.notify();
        } else {
            self.dismiss(window, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.innermost();
        match self.selected[level] {
            // A submenu opened by the mouse: its first row.
            None if level > 0 => self.step(1, cx),
            None => {}
            Some(ix) => {
                if matches!(
                    self.level_items(level).get(ix),
                    Some(MenuItem::Submenu { .. })
                ) {
                    self.open_selected_submenu(cx);
                } else {
                    self.activate(level, ix, window, cx);
                }
            }
        }
    }

    /// Runs row `ix` of `level`: an action closes the menu and is
    /// dispatched where the focus was; a submenu opens.
    fn activate(&mut self, level: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.level_items(level).get(ix) else {
            return;
        };
        match item {
            MenuItem::Action {
                action,
                enabled: true,
                ..
            } => {
                let action = action.boxed_clone();
                self.dismiss(window, cx);
                // The focus is back where it was; the action goes there.
                window.dispatch_action(action, cx);
            }
            MenuItem::Submenu { .. }
                if self.selected.get(level + 1).is_none() || self.selected[level] != Some(ix) =>
            {
                self.select(level, ix, cx);
            }
            _ => {}
        }
    }

    /// The open levels with their rows, for tests and agents.
    pub fn state_json(&self) -> Json {
        let levels: Vec<Json> = (0..self.selected.len())
            .map(|level| {
                let items: Vec<Json> = self
                    .level_items(level)
                    .iter()
                    .map(|item| match item {
                        MenuItem::Separator => json!({ "separator": true }),
                        MenuItem::Action {
                            label,
                            enabled,
                            action,
                            ..
                        } => json!({
                            "label": label.as_ref(),
                            "enabled": enabled,
                            "command": action.name(),
                        }),
                        MenuItem::Submenu { label, .. } => json!({
                            "label": label.as_ref(),
                            "enabled": item.selectable(),
                            "submenu": true,
                        }),
                    })
                    .collect();
                json!({ "items": items, "selected": self.selected[level] })
            })
            .collect();
        json!({ "levels": levels })
    }

    fn render_level(&self, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let items = self.level_items(level);
        let with_icons = items.iter().any(|i| i.icon_name().is_some());
        let selected = self.selected[level];
        let selection = theme.color(UiRole::Selection);
        let text = theme.color(UiRole::Text);
        let disabled = theme.color(UiRole::TextDisabled);
        let line = theme.color(UiRole::PopupBorder);
        let item_height = item_height(cx);
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(ix, item)| {
                let id = SharedString::from(format!("menu-{level}-{ix}"));
                if let MenuItem::Separator = item {
                    return div()
                        .id(id)
                        .h(px(SEPARATOR_HEIGHT))
                        .pt(px(SEPARATOR_INDENT))
                        .px(px(SEPARATOR_X))
                        .child(div().h(px(1.0)).w_full().bg(line))
                        .into_any_element();
                }
                let enabled = item.selectable();
                let is_selected = selected == Some(ix);
                let label = item.label().cloned().unwrap_or_default();
                let selector = format!("menu-item:{label}");
                let submenu = matches!(item, MenuItem::Submenu { .. });
                div()
                    .id(id)
                    .debug_selector(move || selector.clone())
                    .h(px(item_height))
                    .px(px(SELECTION_X))
                    .py(px(SELECTION_Y))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .px(px(CONTENT_X))
                            .rounded(px(SELECTION_RADIUS))
                            .when(is_selected, |d| d.bg(selection))
                            .text_color(if enabled { text } else { disabled })
                            .when(with_icons, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .w(px(ICON + ICON_GAP))
                                        .children(item.icon_name().map(Icon::new)),
                                )
                            })
                            .child(div().flex_1().whitespace_nowrap().child(label))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(TRAILING))
                                    .flex()
                                    .justify_end()
                                    .when(submenu, |d| d.child(Icon::new(IconName::CHEVRON_RIGHT))),
                            ),
                    )
                    .when(enabled, |d| {
                        d.on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.selected.get(level).copied().flatten() != Some(ix)
                                || this.selected.len() <= level
                            {
                                this.select(level, ix, cx);
                            }
                        }))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.activate(level, ix, window, cx);
                            },
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .id(("menu-level", level))
            .debug_selector(move || format!("menu-level-{level}"))
            .occlude()
            .flex()
            .flex_col()
            .py(px(PAD_Y))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, SHADOW_ALPHA),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(SHADOW_BLUR),
                spread_radius: px(0.0),
            }])
            .children(rows)
            .into_any_element()
    }

    /// Distance from the top of `level` to the top of its row `ix`.
    fn row_top(&self, level: usize, ix: usize, item_height: f32) -> f32 {
        self.level_items(level)[..ix]
            .iter()
            .map(|item| item.height(item_height))
            .sum()
    }
}

/// Height of an item row: a list row of the theme and the selection's
/// outer insets.
fn item_height(cx: &App) -> f32 {
    f32::from(cx.theme().metric(Metric::RowHeight)) + 2.0 * SELECTION_Y
}

impl Render for ContextMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let family = cx.theme().ui_family();
        let size = cx.theme().ui_size();
        let item_height = item_height(cx);
        let mut tops = vec![0.0];
        for level in 1..self.selected.len() {
            let parent = self.selected[level - 1].unwrap_or(0);
            tops.push(tops[level - 1] + self.row_top(level - 1, parent, item_height));
        }
        let levels: Vec<AnyElement> = (0..self.selected.len())
            .map(|level| {
                div()
                    .mt(px(tops[level]))
                    .child(self.render_level(level, cx))
                    .into_any_element()
            })
            .collect();
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::open_submenu))
            .on_action(cx.listener(Self::close_submenu))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .flex()
            .flex_row()
            .items_start()
            .font_family(family)
            .text_size(size)
            .children(levels)
    }
}

/// Draws an open menu at `position` (window coordinates) above everything
/// else, with a backdrop over the window that closes it on a click.
pub fn overlay(menu: &Entity<ContextMenu>, position: Point<Pixels>, window: &Window) -> AnyElement {
    let size = window.viewport_size();
    let backdrop_menu = menu.clone();
    deferred(
        anchored().position(point(px(0.0), px(0.0))).child(
            div()
                .id("menu-backdrop")
                .occlude()
                .w(size.width)
                .h(size.height)
                .on_mouse_down(MouseButton::Left, {
                    let menu = backdrop_menu.clone();
                    move |_, window, cx| menu.update(cx, |m, cx| m.dismiss(window, cx))
                })
                .on_mouse_down(MouseButton::Right, {
                    let menu = backdrop_menu.clone();
                    move |_, window, cx| menu.update(cx, |m, cx| m.dismiss(window, cx))
                })
                .child(
                    anchored()
                        .position(position)
                        .snap_to_window_with_margin(px(4.0))
                        .child(menu.clone()),
                ),
        ),
    )
    .with_priority(1)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    gpui::actions!(menu_test, [First, Second]);

    /// The element that had the focus: it records the actions it gets.
    struct Origin {
        focus: FocusHandle,
        got: Vec<&'static str>,
        menu: Option<Entity<ContextMenu>>,
    }

    impl Render for Origin {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &First, _, _| this.got.push("first")))
                .on_action(cx.listener(|this, _: &Second, _, _| this.got.push("second")))
                .size_full()
                .children(self.menu.clone())
        }
    }

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::action("First", First).enabled(false),
            MenuItem::separator(),
            MenuItem::action("Second", Second),
            MenuItem::submenu("More", vec![MenuItem::action("First", First)]),
        ]
    }

    fn open(
        cx: &mut TestAppContext,
    ) -> (Entity<Origin>, Entity<ContextMenu>, &mut VisualTestContext) {
        cx.update(|cx| crate::theme::init(cx, themes::ThemeKind::Dark));
        let (origin, cx) = cx.add_window_view(|_, cx| Origin {
            focus: cx.focus_handle(),
            got: Vec::new(),
            menu: None,
        });
        let menu = cx.update(|window, cx| {
            let focus = origin.read(cx).focus.clone();
            window.focus(&focus);
            ContextMenu::build(items(), window, cx)
        });
        origin.update(cx, |o, _| o.menu = Some(menu.clone()));
        // Actions reach what the last drawn frame has.
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        (origin, menu, cx)
    }

    fn selected(menu: &Entity<ContextMenu>, cx: &mut VisualTestContext) -> Vec<Option<usize>> {
        menu.read_with(cx, |m, _| m.selected.clone())
    }

    #[gpui::test]
    fn keys_skip_separators_and_disabled_rows_and_wrap(cx: &mut TestAppContext) {
        let (_, menu, cx) = open(cx);
        cx.dispatch_action(SelectNext);
        assert_eq!(selected(&menu, cx), [Some(2)]);
        cx.dispatch_action(SelectNext);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        cx.dispatch_action(SelectNext);
        assert_eq!(selected(&menu, cx), [Some(2)]);
        cx.dispatch_action(SelectPrevious);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        // Enter on a submenu row opens it with its first row selected.
        cx.dispatch_action(Confirm);
        assert_eq!(selected(&menu, cx), [Some(3), Some(0)]);
        cx.dispatch_action(CloseSubmenu);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        cx.dispatch_action(OpenSubmenu);
        assert_eq!(selected(&menu, cx), [Some(3), Some(0)]);
        cx.dispatch_action(Cancel);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        let state = menu.read_with(cx, |m, _| m.state_json());
        assert_eq!(state["levels"][0]["items"][0]["enabled"], json!(false));
        assert_eq!(state["levels"][0]["items"][1]["separator"], json!(true));
        assert_eq!(state["levels"][0]["items"][3]["submenu"], json!(true));
    }

    #[gpui::test]
    fn a_chosen_action_goes_where_the_focus_was(cx: &mut TestAppContext) {
        let (origin, menu, cx) = open(cx);
        let dismissed = Rc::new(Cell::new(false));
        let flag = dismissed.clone();
        cx.update(|_, cx| {
            cx.subscribe(&menu, move |_, _: &DismissEvent, _| flag.set(true))
                .detach()
        });
        cx.dispatch_action(SelectNext);
        cx.dispatch_action(Confirm);
        cx.run_until_parked();
        assert!(dismissed.get());
        assert_eq!(origin.read_with(cx, |o, _| o.got.clone()), ["second"]);
        let focused = cx.update(|window, cx| origin.read(cx).focus.is_focused(window));
        assert!(focused, "the focus went back");
    }

    #[gpui::test]
    fn escape_closes_the_menu_and_runs_nothing(cx: &mut TestAppContext) {
        let (origin, menu, cx) = open(cx);
        cx.dispatch_action(SelectNext);
        cx.dispatch_action(Cancel);
        cx.run_until_parked();
        assert!(menu.read_with(cx, |m, _| m.dismissed));
        assert!(origin.read_with(cx, |o, _| o.got.is_empty()));
    }
}
