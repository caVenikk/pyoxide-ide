//! Context menus and list popups, as the reference IDE's New UI draws them:
//! a rounded popup with a border and a shadow, rows with an optional icon
//! column, rounded selection, separators and submenus that open to the
//! right of their item.
//!
//! A component builds a context menu with [`ContextMenu::build`] where the
//! person right-clicked (or from a command), keeps the entity while it is
//! open and draws it with [`overlay`]. A list popup (a popup with a title,
//! a preselected row and speed search, such as a status bar widget's
//! popup) is built with [`ContextMenu::popup`] and drawn with [`overlay_at`]
//! to open from any corner. The menu takes the focus and gives it back to
//! the element that had it when it closes; the chosen item's action is then
//! dispatched there, exactly like a key binding, so menu items are UI
//! commands. The menu emits [`DismissEvent`] when it closes.
//!
//! Keyboard: Up and Down move through the innermost open level, Right and
//! Enter open a submenu, Left and Escape close it, Escape on the menu
//! itself closes the menu, Enter runs the selected item. A click outside
//! the menu closes it; so does deactivating the window. With speed search,
//! typing filters the rows of the popup (Backspace deletes, Escape clears
//! the filter before it closes the popup).

use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Corner, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Window, anchored, deferred, div,
    point, prelude::*, px,
};
use name_match::{Matcher, MatchingMode};
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
        "Clears the speed search, else closes the innermost open submenu, or the menu.";
}

/// Key context of an open context menu.
pub const KEY_CONTEXT: &str = "ContextMenu";

/// How the rows are laid out: as a popup menu (context menus) or as a list
/// popup (a popup with a title).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Metrics {
    /// Selection background inside a row, from the sides and the row's top
    /// and bottom.
    selection_x: f32,
    selection_y: f32,
    /// Content inside the selection background.
    content_x: f32,
    /// A separator line's distance from the sides.
    separator_x: f32,
    /// Space above the first row (without a title) and below the last.
    pad_top: f32,
    pad_bottom: f32,
}

// Metrics of the reference IDE's popup menus in the New UI (see NOTICE).
// An item row is a list row (`List.rowHeight`, the theme's row height)
// plus the selection's outer insets above and below (`IdeaMenuUI.
// patchPreferredSize`): selection `PopupMenu.Selection.outerInsets`
// (1, 7), content `PopupMenu.Selection.innerInsets` (0, 6) of the New UI,
// separators `withToEdge` 10 from both sides, `PopupMenu.borderInsets`
// (6, 1, 6, 1) above the first and below the last row (the 1 is the
// border).
const MENU: Metrics = Metrics {
    selection_x: 7.0,
    selection_y: 1.0,
    content_x: 6.0,
    separator_x: 10.0,
    pad_top: 6.0,
    pad_bottom: 6.0,
};

// List popups (`ListPopupImpl`) in the New UI: rows of `List.rowHeight`
// with the selection `Popup.Selection.leftRightInset` 8 from the sides and
// the content `Popup.Selection.innerInsets` (0, 8) inside it
// (`PopupUtil.configListRendererFixedHeight`), separators of
// `Popup.separatorInsets` (4, 8) (`GroupHeaderSeparator`), and the body
// `Popup.Body.topInsetNoHeader` 8 above the rows when there is no title,
// `Popup.Body.bottomInsetNoAd` 8 below them (`PopupUtil.getListInsets`).
const LIST: Metrics = Metrics {
    selection_x: 8.0,
    selection_y: 0.0,
    content_x: 8.0,
    separator_x: 8.0,
    pad_top: 8.0,
    pad_bottom: 8.0,
};

/// Radius of the selection background: `PopupMenu.Selection.arc` and
/// `Popup.Selection.arc` 8 are diameters.
const SELECTION_RADIUS: f32 = 4.0;
/// Separator row: `PopupMenuSeparator.height` 9 (`Popup.separatorInsets`
/// 4 + 1 + 4 in list popups), a line of 1 at 4.
const SEPARATOR_HEIGHT: f32 = 9.0;
const SEPARATOR_INDENT: f32 = 4.0;
/// Popup corners: `PopupMenu.borderCornerRadius` 8.
const RADIUS: f32 = 8.0;
/// Icon column (16 px icons) and the gap to the label: Swing's menu item
/// `defaultTextIconGap` 4; in list popups `ActionsList.icon.gap` 6 less 2.
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 4.0;
/// Room after the label (`BegMenuItemUI.getPreferredSize`: the gutter of
/// 18 and four text gaps); a submenu's arrow sits at its right end.
const TRAILING: f32 = 18.0 + 4.0 * ICON_GAP;
/// The title of a list popup in the New UI (`TitlePanel.setPopupTitle`):
/// bold and centred (`TitlePanel`: a centred label), `Popup.Header.insets`
/// (12, 10, 10, 10).
const TITLE_TOP: f32 = 12.0;
const TITLE_X: f32 = 10.0;
const TITLE_BOTTOM: f32 = 10.0;
/// The speed search field under the title (`AbstractPopup.
/// customizeSearchFieldLook`): `Popup.SearchField.borderInsets` (4, 12)
/// around it, a line below it, `Popup.SearchField.inputInsets`
/// (4, 8, 8, 2) inside.
const SEARCH_MARGIN_Y: f32 = 4.0;
const SEARCH_MARGIN_X: f32 = 12.0;
const SEARCH_TOP: f32 = 4.0;
const SEARCH_LEFT: f32 = 8.0;
const SEARCH_BOTTOM: f32 = 8.0;
/// A row's buttons (`ActionButton` in a list row) and the gap before a
/// secondary text (`emptyLeft(6)`).
const INLINE_BUTTON: f32 = 20.0;
const SECONDARY_GAP: f32 = 6.0;
/// Characters besides letters and digits that speed search takes
/// (`SpeedSearch.PUNCTUATION_MARKS`); a space only after the first char.
const PUNCTUATION_MARKS: &str = "*_-+\"'/.#$>: ,;?!@%^&";

/// What a row can have besides its label (the reference's list popup
/// rows): a secondary text after the label, a faded icon, buttons at its
/// end shown while it is selected, and whether it shows only while the
/// speed search filters.
#[derive(Default)]
pub struct RowExtra {
    /// Grey text right after the label (`secondaryLabel`,
    /// `MenuItem.acceleratorForeground`).
    pub secondary: Option<SharedString>,
    /// Opacity of the icon (a temporary run configuration's is 0.45).
    pub icon_alpha: Option<f32>,
    /// Buttons at the end of the row (`ActionUtil.INLINE_ACTIONS`).
    pub inline: Vec<InlineButton>,
    /// Shown only while the speed search filters (a row hidden behind a
    /// toggle, or a duplicate found by search).
    pub hidden: bool,
}

impl Clone for RowExtra {
    fn clone(&self) -> Self {
        Self {
            secondary: self.secondary.clone(),
            icon_alpha: self.icon_alpha,
            inline: self.inline.clone(),
            hidden: self.hidden,
        }
    }
}

/// A button at the end of a row: runs its action (where the focus was)
/// and closes the menu. Without an action it is drawn disabled.
pub struct InlineButton {
    pub icon: IconName,
    pub action: Option<Box<dyn Action>>,
    pub tooltip: SharedString,
}

impl InlineButton {
    pub fn new(icon: IconName, tooltip: impl Into<SharedString>, action: impl Action) -> Self {
        Self {
            icon,
            action: Some(Box::new(action)),
            tooltip: tooltip.into(),
        }
    }

    pub fn disabled(icon: IconName, tooltip: impl Into<SharedString>) -> Self {
        Self {
            icon,
            action: None,
            tooltip: tooltip.into(),
        }
    }
}

impl Clone for InlineButton {
    fn clone(&self) -> Self {
        Self {
            icon: self.icon,
            action: self.action.as_ref().map(|a| a.boxed_clone()),
            tooltip: self.tooltip.clone(),
        }
    }
}

/// One row of a menu.
pub enum MenuItem {
    /// Runs `action` where the focus was before the menu opened. A row
    /// without an action is shown disabled.
    Action {
        label: SharedString,
        icon: Option<IconName>,
        action: Option<Box<dyn Action>>,
        enabled: bool,
        /// What the item does, for agents and tests (the reference keeps it
        /// in the action's presentation).
        description: Option<SharedString>,
        extra: RowExtra,
    },
    /// Opens `items` next to it while it is selected; with an `action`
    /// (the reference's "perform group") a click or Enter runs the action
    /// instead of opening the submenu.
    Submenu {
        label: SharedString,
        icon: Option<IconName>,
        items: Vec<MenuItem>,
        enabled: bool,
        action: Option<Box<dyn Action>>,
        description: Option<SharedString>,
        extra: RowExtra,
    },
    /// A separator with a title (`Separator.create(text)`): "Pinned",
    /// "Recent Configurations".
    Header(SharedString),
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
                description,
                extra,
            } => Self::Action {
                label: label.clone(),
                icon: *icon,
                action: action.as_ref().map(|a| a.boxed_clone()),
                enabled: *enabled,
                description: description.clone(),
                extra: extra.clone(),
            },
            Self::Submenu {
                label,
                icon,
                items,
                enabled,
                action,
                description,
                extra,
            } => Self::Submenu {
                label: label.clone(),
                icon: *icon,
                items: items.clone(),
                enabled: *enabled,
                action: action.as_ref().map(|a| a.boxed_clone()),
                description: description.clone(),
                extra: extra.clone(),
            },
            Self::Header(label) => Self::Header(label.clone()),
            Self::Separator => Self::Separator,
        }
    }
}

impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action { label, action, .. } => match action {
                Some(action) => write!(f, "Action({label:?}, {})", action.name()),
                None => write!(f, "Action({label:?})"),
            },
            Self::Submenu { label, items, .. } => write!(f, "Submenu({label:?}, {items:?})"),
            Self::Header(label) => write!(f, "Header({label:?})"),
            Self::Separator => f.write_str("Separator"),
        }
    }
}

impl MenuItem {
    pub fn action(label: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Action {
            label: label.into(),
            icon: None,
            action: Some(Box::new(action)),
            enabled: true,
            description: None,
            extra: RowExtra::default(),
        }
    }

    /// A row that is shown but cannot be chosen (its action is not there
    /// yet).
    pub fn unavailable(label: impl Into<SharedString>) -> Self {
        Self::Action {
            label: label.into(),
            icon: None,
            action: None,
            enabled: false,
            description: None,
            extra: RowExtra::default(),
        }
    }

    pub fn submenu(label: impl Into<SharedString>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            icon: None,
            items,
            enabled: true,
            action: None,
            description: None,
            extra: RowExtra::default(),
        }
    }

    pub fn separator() -> Self {
        Self::Separator
    }

    /// A separator with a title.
    pub fn header(label: impl Into<SharedString>) -> Self {
        Self::Header(label.into())
    }

    pub fn icon(mut self, name: IconName) -> Self {
        match &mut self {
            Self::Action { icon, .. } | Self::Submenu { icon, .. } => *icon = Some(name),
            Self::Header(_) | Self::Separator => {}
        }
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        match &mut self {
            Self::Action { enabled, .. } | Self::Submenu { enabled, .. } => *enabled = on,
            Self::Header(_) | Self::Separator => {}
        }
        self
    }

    /// What the item does (see [`MenuItem::Action`]).
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        match &mut self {
            Self::Action { description, .. } | Self::Submenu { description, .. } => {
                *description = Some(text.into())
            }
            Self::Header(_) | Self::Separator => {}
        }
        self
    }

    /// A submenu row that runs `action` when it is clicked or chosen with
    /// Enter (its submenu opens while it is selected).
    pub fn perform(mut self, run: impl Action) -> Self {
        if let Self::Submenu { action, .. } = &mut self {
            *action = Some(Box::new(run));
        }
        self
    }

    /// Grey text after the label.
    pub fn secondary(mut self, text: impl Into<SharedString>) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.secondary = Some(text.into());
        }
        self
    }

    /// The icon's opacity.
    pub fn icon_alpha(mut self, alpha: f32) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.icon_alpha = Some(alpha);
        }
        self
    }

    /// A button at the end of the row.
    pub fn inline(mut self, button: InlineButton) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.inline.push(button);
        }
        self
    }

    /// Shown only while the speed search filters.
    pub fn hidden(mut self, hidden: bool) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.hidden = hidden;
        }
        self
    }

    pub fn label(&self) -> Option<&SharedString> {
        match self {
            Self::Action { label, .. } | Self::Submenu { label, .. } => Some(label),
            Self::Header(_) | Self::Separator => None,
        }
    }

    fn extra(&self) -> Option<&RowExtra> {
        match self {
            Self::Action { extra, .. } | Self::Submenu { extra, .. } => Some(extra),
            Self::Header(_) | Self::Separator => None,
        }
    }

    fn extra_mut(&mut self) -> Option<&mut RowExtra> {
        match self {
            Self::Action { extra, .. } | Self::Submenu { extra, .. } => Some(extra),
            Self::Header(_) | Self::Separator => None,
        }
    }

    fn is_hidden(&self) -> bool {
        self.extra().is_some_and(|e| e.hidden)
    }

    fn icon_name(&self) -> Option<IconName> {
        match self {
            Self::Action { icon, .. } | Self::Submenu { icon, .. } => *icon,
            Self::Header(_) | Self::Separator => None,
        }
    }

    /// Has a submenu to open.
    fn has_submenu(&self) -> bool {
        matches!(self, Self::Submenu { items, .. } if !items.is_empty())
    }

    /// The action a click or Enter runs (a submenu row's only with
    /// `perform`).
    fn run_action(&self) -> Option<&dyn Action> {
        match self {
            Self::Action {
                action: Some(action),
                enabled: true,
                ..
            }
            | Self::Submenu {
                action: Some(action),
                enabled: true,
                ..
            } => Some(action.as_ref()),
            _ => None,
        }
    }

    /// Can be selected: not a separator, not disabled.
    fn selectable(&self) -> bool {
        match self {
            Self::Action {
                enabled, action, ..
            } => *enabled && action.is_some(),
            Self::Submenu {
                items,
                enabled,
                action,
                ..
            } => *enabled && (!items.is_empty() || action.is_some()),
            Self::Header(_) | Self::Separator => false,
        }
    }

    fn height(&self, item_height: f32) -> f32 {
        match self {
            Self::Separator => SEPARATOR_HEIGHT,
            _ => item_height,
        }
    }
}

/// How [`ContextMenu::popup`] opens a list popup.
#[derive(Debug, Clone, Default)]
pub struct PopupOptions {
    /// The title above the rows.
    pub title: Option<SharedString>,
    /// Typing filters the rows (the reference's speed search).
    pub speed_search: bool,
    /// The row selected when the popup opens (the reference's preselect
    /// condition); without one, or when it cannot be selected, the first
    /// row that can.
    pub selected: Option<usize>,
    /// Narrowest and widest the popup is drawn (the run widget's dropdown:
    /// 310 to 500).
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
}

/// An open context menu or list popup.
pub struct ContextMenu {
    focus: FocusHandle,
    items: Vec<MenuItem>,
    /// The selected row of each open level: the menu itself first, then
    /// the submenu of its selected row, and so on.
    selected: Vec<Option<usize>>,
    /// Had the focus before; gets it back and runs the chosen action.
    origin: Option<FocusHandle>,
    dismissed: bool,
    metrics: Metrics,
    title: Option<SharedString>,
    speed_search: bool,
    /// The speed search's text and its matcher (`*` and the text, case
    /// ignored, as the reference builds it).
    filter: String,
    matcher: Option<Matcher>,
    min_width: Option<f32>,
    max_width: Option<f32>,
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
        Self::open(items, MENU, PopupOptions::default(), window, cx)
    }

    /// Opens a list popup of `items` (a title, a preselected row, speed
    /// search) and gives it the focus, like [`ContextMenu::build`].
    pub fn popup(
        items: Vec<MenuItem>,
        options: PopupOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::open(items, LIST, options, window, cx)
    }

    fn open(
        items: Vec<MenuItem>,
        metrics: Metrics,
        options: PopupOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let origin = window.focused(cx);
        let list = metrics == LIST;
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
            let mut menu = Self {
                focus,
                items,
                selected: vec![None],
                origin,
                dismissed: false,
                metrics,
                title: options.title,
                speed_search: options.speed_search,
                filter: String::new(),
                matcher: None,
                min_width: options.min_width,
                max_width: options.max_width,
                _subscriptions: subscriptions,
            };
            if list {
                menu.selected[0] = options
                    .selected
                    .filter(|&ix| menu.items.get(ix).is_some_and(MenuItem::selectable))
                    .or_else(|| menu.first_selectable());
            }
            menu
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

    /// The menu was closed.
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    fn level_items(&self, level: usize) -> &[MenuItem] {
        let mut items = self.items.as_slice();
        for selected in &self.selected[..level] {
            match selected.and_then(|i| items.get(i)) {
                Some(MenuItem::Submenu { items: sub, .. }) if !sub.is_empty() => items = sub,
                _ => return &[],
            }
        }
        items
    }

    fn innermost(&self) -> usize {
        self.selected.len() - 1
    }

    // ---- speed search -----------------------------------------------------

    fn matches(&self, item: &MenuItem) -> bool {
        match (&self.matcher, item.label()) {
            (None, _) => true,
            (Some(m), Some(label)) => m.matches(label),
            (Some(_), None) => false,
        }
    }

    /// Which rows of `level` are shown: all without a filter; with one the
    /// matching rows, and a separator before a shown row when a shown row
    /// came before it (`ListPopupModel.rebuildLists`, leading and repeated
    /// separators dropped).
    fn shown(&self, level: usize) -> Vec<bool> {
        let items = self.level_items(level);
        if level > 0 || self.matcher.is_none() {
            return items.iter().map(|i| !i.is_hidden()).collect();
        }
        let mut shown = vec![false; items.len()];
        let mut pending = None;
        let mut any = false;
        for (ix, item) in items.iter().enumerate() {
            if let MenuItem::Header(_) = item {
                // Titles go while filtering; a line stays between groups.
                pending = pending.or(Some(ix));
            } else if let MenuItem::Separator = item {
                pending = Some(ix);
            } else if self.matches(item) {
                if let Some(sep) = pending.take()
                    && any
                {
                    shown[sep] = true;
                }
                shown[ix] = true;
                any = true;
            }
        }
        shown
    }

    fn first_selectable(&self) -> Option<usize> {
        let shown = self.shown(0);
        self.items
            .iter()
            .enumerate()
            .position(|(ix, item)| shown[ix] && item.selectable())
    }

    fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.matcher = (!self.filter.is_empty())
            .then(|| Matcher::new(&format!("*{}", self.filter), MatchingMode::IgnoreCase));
        self.selected.truncate(1);
        // The best match: the row whose text is the filter, else the first
        // that starts with it, else the first row that can be selected
        // (`ListPopupImpl.selectBestMatch`).
        let upper = self.filter.to_uppercase();
        let shown = self.shown(0);
        let candidates = || {
            self.items.iter().enumerate().filter(|(ix, item)| {
                shown[*ix] && !matches!(item, MenuItem::Separator | MenuItem::Header(_))
            })
        };
        let label = |item: &MenuItem| item.label().map(|l| l.to_uppercase()).unwrap_or_default();
        let best = if upper.is_empty() {
            None
        } else {
            candidates()
                .find(|(_, item)| label(item) == upper)
                .or_else(|| candidates().find(|(_, item)| label(item).starts_with(&upper)))
                .map(|(ix, _)| ix)
        };
        self.selected[0] = best
            .filter(|&ix| self.items[ix].selectable())
            .or_else(|| self.first_selectable());
        cx.notify();
    }

    /// A typed char goes to the speed search, Backspace deletes the last
    /// one (`SpeedSearch.processKeyEvent`).
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.speed_search || self.innermost() > 0 {
            return;
        }
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform || k.modifiers.alt || k.modifiers.function {
            return;
        }
        if k.key == "backspace" {
            if !self.filter.is_empty() {
                let mut filter = self.filter.clone();
                filter.pop();
                self.set_filter(filter, cx);
                cx.stop_propagation();
            }
            return;
        }
        let Some(text) = k.key_char.as_deref() else {
            return;
        };
        let mut filter = self.filter.clone();
        for c in text.chars() {
            let starts_with_space = filter.is_empty() && c.is_whitespace();
            if c.is_alphanumeric() || (!starts_with_space && PUNCTUATION_MARKS.contains(c)) {
                filter.push(c);
            }
        }
        if filter != self.filter {
            self.set_filter(filter, cx);
            cx.stop_propagation();
        }
    }

    // ---- selection and keys -------------------------------------------------

    /// Selects row `ix` of `level`, closing deeper levels; a submenu row
    /// opens its submenu (with nothing selected yet).
    fn select(&mut self, level: usize, ix: usize, cx: &mut Context<Self>) {
        let Some(item) = self.level_items(level).get(ix) else {
            return;
        };
        if !item.selectable() {
            return;
        }
        let submenu = item.has_submenu();
        self.selected.truncate(level + 1);
        self.selected[level] = Some(ix);
        if submenu {
            self.selected.push(None);
        }
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let level = self.innermost();
        let shown = self.shown(level);
        let items = self.level_items(level);
        let n = items.len() as isize;
        if n == 0 {
            return;
        }
        let start = self.selected[level].map_or(if delta > 0 { -1 } else { n }, |i| i as isize);
        let mut i = start;
        for _ in 0..n {
            i = (i + delta).rem_euclid(n);
            if shown[i as usize] && items[i as usize].selectable() {
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
        if !self
            .level_items(level)
            .get(ix)
            .is_some_and(MenuItem::has_submenu)
        {
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
        } else if !self.filter.is_empty() {
            self.set_filter(String::new(), cx);
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
                let item = self.level_items(level).get(ix);
                if item.is_some_and(|i| i.has_submenu() && i.run_action().is_none()) {
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
        if let Some(action) = item.run_action() {
            let action = action.boxed_clone();
            self.dismiss(window, cx);
            // The focus is back where it was; the action goes there.
            window.dispatch_action(action, cx);
            return;
        }
        match item {
            MenuItem::Submenu { .. }
                if self.selected.get(level + 1).is_none() || self.selected[level] != Some(ix) =>
            {
                self.select(level, ix, cx);
            }
            _ => {}
        }
    }

    /// Runs a row's button: closes the menu and dispatches its action where
    /// the focus was.
    fn run_inline(
        &mut self,
        level: usize,
        ix: usize,
        button: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self
            .level_items(level)
            .get(ix)
            .and_then(|i| i.extra())
            .and_then(|e| e.inline.get(button))
            .and_then(|b| b.action.as_ref())
            .map(|a| a.boxed_clone())
        else {
            return;
        };
        self.dismiss(window, cx);
        window.dispatch_action(action, cx);
    }

    /// The open levels with their rows, for tests and agents.
    pub fn state_json(&self) -> Json {
        let levels: Vec<Json> = (0..self.selected.len())
            .map(|level| {
                let shown = self.shown(level);
                let items: Vec<Json> = self
                    .level_items(level)
                    .iter()
                    .enumerate()
                    .map(|(ix, item)| {
                        let mut row = match item {
                            MenuItem::Separator => json!({ "separator": true }),
                            MenuItem::Header(label) => json!({ "header": label.as_ref() }),
                            MenuItem::Action {
                                label,
                                action,
                                description,
                                ..
                            } => json!({
                                "label": label.as_ref(),
                                "enabled": item.selectable(),
                                "command": action.as_ref().map(|a| a.name()),
                                "description": description.as_ref().map(|d| d.as_ref()),
                            }),
                            MenuItem::Submenu {
                                label,
                                action,
                                description,
                                ..
                            } => json!({
                                "label": label.as_ref(),
                                "enabled": item.selectable(),
                                "submenu": true,
                                "command": action.as_ref().map(|a| a.name()),
                                "description": description.as_ref().map(|d| d.as_ref()),
                            }),
                        };
                        if let Some(extra) = item.extra() {
                            if let Some(text) = &extra.secondary {
                                row["secondary"] = json!(text.as_ref());
                            }
                            if let Some(alpha) = extra.icon_alpha {
                                row["icon_alpha"] = json!(alpha);
                            }
                            if !extra.inline.is_empty() {
                                row["inline"] = extra
                                    .inline
                                    .iter()
                                    .map(|b| {
                                        json!({
                                            "tooltip": b.tooltip.as_ref(),
                                            "icon": b.icon.0,
                                            "enabled": b.action.is_some(),
                                            "command": b.action.as_ref().map(|a| a.name()),
                                        })
                                    })
                                    .collect();
                            }
                        }
                        if !shown[ix] {
                            row["hidden"] = json!(true);
                        }
                        row
                    })
                    .collect();
                json!({ "items": items, "selected": self.selected[level] })
            })
            .collect();
        json!({
            "title": self.title.as_ref().map(|t| t.as_ref()),
            "filter": self.filter,
            "levels": levels,
        })
    }

    // ---- drawing ------------------------------------------------------------

    fn render_level(&self, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let m = self.metrics;
        let items = self.level_items(level);
        let shown = self.shown(level);
        let with_icons = items.iter().any(|i| i.icon_name().is_some());
        let selected = self.selected[level];
        let selection = theme.color(UiRole::Selection);
        let text = theme.color(UiRole::Text);
        let disabled = theme.color(UiRole::TextDisabled);
        let line = theme.color(UiRole::PopupBorder);
        let match_bg = theme.color(UiRole::SearchMatch);
        let item_height = self.item_height(cx);
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter(|(ix, _)| shown[*ix])
            .map(|(ix, item)| {
                let id = SharedString::from(format!("menu-{level}-{ix}"));
                if let MenuItem::Header(title) = item {
                    let selector = format!("menu-header:{title}");
                    let first = !items[..ix].iter().enumerate().any(|(i, _)| shown[i]);
                    return div()
                        .id(id)
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_col()
                        .justify_end()
                        .h(px(item_height))
                        .px(px(m.separator_x))
                        .when(!first, |d| {
                            d.child(div().flex_none().h(px(1.0)).w_full().bg(line))
                        })
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .px(px(m.content_x))
                                .text_color(disabled)
                                .whitespace_nowrap()
                                .child(title.clone()),
                        )
                        .into_any_element();
                }
                if let MenuItem::Separator = item {
                    let selector = format!("menu-separator-{level}-{ix}");
                    // A column, so that the line stretches across.
                    return div()
                        .id(id)
                        .flex()
                        .flex_col()
                        .h(px(SEPARATOR_HEIGHT))
                        .pt(px(SEPARATOR_INDENT))
                        .px(px(m.separator_x))
                        .child(
                            div()
                                .debug_selector(move || selector.clone())
                                .h(px(1.0))
                                .w_full()
                                .bg(line),
                        )
                        .into_any_element();
                }
                let enabled = item.selectable();
                let is_selected = selected == Some(ix);
                let label = item.label().cloned().unwrap_or_default();
                let selector = format!("menu-item:{label}");
                let submenu = item.has_submenu();
                let extra = item.extra();
                let secondary = extra.and_then(|e| e.secondary.clone());
                let icon_alpha = extra.and_then(|e| e.icon_alpha).unwrap_or(1.0);
                let inline: Vec<AnyElement> = if is_selected {
                    extra
                        .map(|e| e.inline.as_slice())
                        .unwrap_or(&[])
                        .iter()
                        .enumerate()
                        .map(|(b, button)| {
                            let enabled = button.action.is_some();
                            let tooltip = button.tooltip.clone();
                            let selector = format!("menu-inline:{tooltip}");
                            div()
                                .id(SharedString::from(format!("menu-inline-{level}-{ix}-{b}")))
                                .debug_selector(move || selector.clone())
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(INLINE_BUTTON))
                                .rounded(px(SELECTION_RADIUS))
                                .when(!enabled, |d| d.opacity(crate::components::DISABLED_ALPHA))
                                .child(Icon::new(button.icon))
                                .tooltip(crate::components::Tooltip::text(tooltip, None))
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.run_inline(level, ix, b, window, cx);
                                    }))
                                })
                                .into_any_element()
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let fragments = match (&self.matcher, level) {
                    (Some(matcher), 0) => matcher.find(&label).unwrap_or_default(),
                    _ => Vec::new(),
                };
                let label = StyledText::new(label.clone())
                    .with_highlights(highlights(&label, &fragments, match_bg));
                div()
                    .id(id)
                    .debug_selector(move || selector.clone())
                    .h(px(item_height))
                    .px(px(m.selection_x))
                    .py(px(m.selection_y))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .px(px(m.content_x))
                            .rounded(px(SELECTION_RADIUS))
                            .when(is_selected, |d| d.bg(selection))
                            .text_color(if enabled { text } else { disabled })
                            .when(with_icons, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .w(px(ICON + ICON_GAP))
                                        .when(icon_alpha < 1.0, |d| d.opacity(icon_alpha))
                                        .children(item.icon_name().map(Icon::new)),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .overflow_hidden()
                                    .child(div().flex_none().whitespace_nowrap().child(label))
                                    .children(secondary.map(|text| {
                                        div()
                                            .flex_none()
                                            .pl(px(SECONDARY_GAP))
                                            .whitespace_nowrap()
                                            .text_color(disabled)
                                            .child(text)
                                    })),
                            )
                            .children(inline)
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
        let header = level == 0 && self.title.is_some();
        let (min_width, max_width) = if level == 0 {
            (self.min_width, self.max_width)
        } else {
            (None, None)
        };
        let title = (level == 0).then(|| self.title.clone()).flatten();
        let search = (level == 0 && !self.filter.is_empty()).then(|| {
            div()
                .id("menu-speed-search")
                .debug_selector(|| "menu-speed-search".into())
                .flex_none()
                .mx(px(SEARCH_MARGIN_X))
                .my(px(SEARCH_MARGIN_Y))
                .border_b_1()
                .border_color(line)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(ICON_GAP))
                        .pt(px(SEARCH_TOP))
                        .pl(px(SEARCH_LEFT))
                        .pb(px(SEARCH_BOTTOM))
                        .child(Icon::new(IconName::SEARCH))
                        .text_color(text)
                        .child(div().whitespace_nowrap().child(self.filter.clone())),
                )
        });
        div()
            .id(("menu-level", level))
            .debug_selector(move || format!("menu-level-{level}"))
            .occlude()
            .flex()
            .flex_col()
            .when_some(min_width, |d, w| d.min_w(px(w)))
            .when_some(max_width, |d, w| d.max_w(px(w)))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(crate::components::popup_shadow())
            .children(title.map(|title| {
                div()
                    .debug_selector(|| "menu-title".into())
                    .flex_none()
                    .flex()
                    .justify_center()
                    .pt(px(TITLE_TOP))
                    .pb(px(TITLE_BOTTOM))
                    .px(px(TITLE_X))
                    .font_weight(FontWeight::BOLD)
                    .text_color(text)
                    .whitespace_nowrap()
                    .child(title)
            }))
            .children(search)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(!header, |d| d.pt(px(m.pad_top)))
                    .pb(px(m.pad_bottom))
                    .children(rows),
            )
            .into_any_element()
    }

    /// Height of an item row: a list row of the theme, and in popup menus
    /// the selection's outer insets.
    fn item_height(&self, cx: &App) -> f32 {
        f32::from(cx.theme().metric(Metric::RowHeight)) + 2.0 * self.metrics.selection_y
    }

    /// Distance from the top of `level`'s rows to the top of its row `ix`.
    fn row_top(&self, level: usize, ix: usize, item_height: f32) -> f32 {
        let shown = self.shown(level);
        self.level_items(level)[..ix]
            .iter()
            .enumerate()
            .filter(|(i, _)| shown[*i])
            .map(|(_, item)| item.height(item_height))
            .sum()
    }
}

/// Char ranges of `text` as byte ranges with a background.
fn highlights(
    text: &str,
    ranges: &[std::ops::Range<usize>],
    background: Hsla,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let style = HighlightStyle {
        background_color: Some(background),
        ..HighlightStyle::default()
    };
    ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.min(offsets.len() - 1);
            let b = r.end.min(offsets.len() - 1);
            (b > a).then(|| (offsets[a]..offsets[b], style))
        })
        .collect()
}

impl Render for ContextMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let family = cx.theme().ui_family();
        let size = cx.theme().ui_size();
        let item_height = self.item_height(cx);
        // A submenu lines up with its row: both levels have the same space
        // above their rows, except below a title (about a row high).
        let mut tops = vec![0.0];
        for level in 1..self.selected.len() {
            let parent = self.selected[level - 1].unwrap_or(0);
            let header = if level == 1 && self.title.is_some() {
                TITLE_TOP + item_height + TITLE_BOTTOM - self.metrics.pad_top
            } else {
                0.0
            };
            tops.push(tops[level - 1] + header + self.row_top(level - 1, parent, item_height));
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
            .on_key_down(cx.listener(Self::key_down))
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
    overlay_at(menu, position, Corner::TopLeft, window)
}

/// Like [`overlay`], with the menu's `corner` at `position`: a popup of a
/// status bar widget opens upward with its bottom left corner at the
/// widget's top left (`EditorBasedStatusBarPopup.showPopup`). The menu is
/// moved to stay inside the window.
pub fn overlay_at(
    menu: &Entity<ContextMenu>,
    position: Point<Pixels>,
    corner: Corner,
    window: &Window,
) -> AnyElement {
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
                        .anchor(corner)
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

    gpui::actions!(menu_test, [First, Second, Third]);

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
                .on_action(cx.listener(|this, _: &Third, _, _| this.got.push("third")))
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

    fn open_with(
        cx: &mut TestAppContext,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<ContextMenu>,
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
            build(window, cx)
        });
        origin.update(cx, |o, _| o.menu = Some(menu.clone()));
        // Actions reach what the last drawn frame has.
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        (origin, menu, cx)
    }

    fn open(
        cx: &mut TestAppContext,
    ) -> (Entity<Origin>, Entity<ContextMenu>, &mut VisualTestContext) {
        open_with(cx, |window, cx| ContextMenu::build(items(), window, cx))
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
        assert_eq!(state["title"], Json::Null);
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

    fn popup_items() -> Vec<MenuItem> {
        vec![
            MenuItem::action("Python 3.12 (demo)", First).description("Switch to a"),
            MenuItem::separator(),
            MenuItem::action("uv (demo)", Second),
            MenuItem::action("uv (tools)", Third),
            MenuItem::separator(),
            MenuItem::submenu("Add New Interpreter", vec![]).enabled(false),
            MenuItem::separator(),
            MenuItem::unavailable("Interpreter Settings…"),
        ]
    }

    fn popup(
        cx: &mut TestAppContext,
        selected: Option<usize>,
    ) -> (Entity<Origin>, Entity<ContextMenu>, &mut VisualTestContext) {
        open_with(cx, move |window, cx| {
            let options = PopupOptions {
                title: Some("Python Interpreter".into()),
                speed_search: true,
                selected,
                ..PopupOptions::default()
            };
            ContextMenu::popup(popup_items(), options, window, cx)
        })
    }

    fn shown_labels(menu: &Entity<ContextMenu>, cx: &mut VisualTestContext) -> Vec<String> {
        let state = menu.read_with(cx, |m, _| m.state_json());
        state["levels"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i.get("hidden").is_none())
            .map(|i| i["label"].as_str().unwrap_or("---").to_owned())
            .collect()
    }

    #[gpui::test]
    fn a_popup_has_a_title_and_starts_on_its_preselected_row(cx: &mut TestAppContext) {
        let (_, menu, cx) = popup(cx, Some(3));
        assert_eq!(selected(&menu, cx), [Some(3)]);
        let state = menu.read_with(cx, |m, _| m.state_json());
        assert_eq!(state["title"], json!("Python Interpreter"));
        assert_eq!(
            state["levels"][0]["items"][0]["description"],
            json!("Switch to a")
        );
        assert_eq!(state["levels"][0]["items"][5]["enabled"], json!(false));
        assert_eq!(state["levels"][0]["items"][7]["enabled"], json!(false));
        assert_eq!(state["levels"][0]["items"][7]["command"], Json::Null);
        // Disabled rows are skipped by the keys.
        cx.dispatch_action(SelectNext);
        assert_eq!(selected(&menu, cx), [Some(0)]);
        let title = cx.debug_bounds("menu-title");
        assert!(title.is_some(), "the title is drawn");
        let row = cx.debug_bounds("menu-item:Python 3.12 (demo)").unwrap();
        let line = cx.debug_bounds("menu-separator-0-1").unwrap();
        assert_eq!(line.size.height, px(1.0));
        // The line spans the popup less `Popup.separatorInsets` (8).
        assert_eq!(line.size.width, row.size.width - px(2.0 * LIST.separator_x));
    }

    #[gpui::test]
    fn a_preselected_row_that_cannot_be_chosen_gives_the_first(cx: &mut TestAppContext) {
        let (_, menu, cx) = popup(cx, Some(5));
        assert_eq!(selected(&menu, cx), [Some(0)]);
    }

    #[gpui::test]
    fn typing_filters_a_popup(cx: &mut TestAppContext) {
        let (origin, menu, cx) = popup(cx, Some(0));
        cx.simulate_input("uv");
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "uv");
        assert_eq!(shown_labels(&menu, cx), ["uv (demo)", "uv (tools)"]);
        // The first row that starts with the filter is selected.
        assert_eq!(selected(&menu, cx), [Some(2)]);
        cx.simulate_input(" t");
        assert_eq!(shown_labels(&menu, cx), ["uv (tools)"]);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        cx.simulate_keystrokes("backspace backspace");
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "uv");
        // Separators are kept between rows that are shown.
        cx.simulate_input("x");
        assert!(shown_labels(&menu, cx).is_empty());
        cx.simulate_keystrokes("backspace backspace backspace");
        cx.simulate_input("n");
        assert_eq!(
            shown_labels(&menu, cx),
            [
                "Python 3.12 (demo)",
                "---",
                "Add New Interpreter",
                "---",
                "Interpreter Settings…"
            ]
        );
        // Escape clears the filter first, then closes.
        cx.dispatch_action(Cancel);
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "");
        assert_eq!(shown_labels(&menu, cx).len(), 8);
        cx.simulate_input("tools");
        cx.dispatch_action(Confirm);
        cx.run_until_parked();
        assert_eq!(origin.read_with(cx, |o, _| o.got.clone()), ["third"]);
        assert!(menu.read_with(cx, |m, _| m.is_dismissed()));
    }

    #[gpui::test]
    fn a_leading_space_is_not_typed(cx: &mut TestAppContext) {
        let (_, menu, cx) = popup(cx, None);
        cx.simulate_input(" ");
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "");
        cx.simulate_input("u v");
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "u v");
    }

    #[gpui::test]
    fn a_context_menu_has_no_speed_search(cx: &mut TestAppContext) {
        let (_, menu, cx) = open(cx);
        cx.simulate_input("s");
        assert_eq!(menu.read_with(cx, |m, _| m.filter.clone()), "");
        assert_eq!(selected(&menu, cx), [None]);
    }

    fn rich_items() -> Vec<MenuItem> {
        vec![
            MenuItem::header("Pinned"),
            MenuItem::submenu("alpha", vec![MenuItem::action("Third", Third)])
                .perform(First)
                .secondary("not supported yet")
                .icon(IconName::RUN)
                .icon_alpha(0.45)
                .inline(InlineButton::new(IconName::RUN, "Run 'alpha'", Second))
                .inline(InlineButton::disabled(IconName::DEBUG, "Debug 'alpha'")),
            MenuItem::separator(),
            MenuItem::action("beta", Second),
            MenuItem::action("gamma", Third).hidden(true),
        ]
    }

    fn rich(
        cx: &mut TestAppContext,
    ) -> (Entity<Origin>, Entity<ContextMenu>, &mut VisualTestContext) {
        open_with(cx, |window, cx| {
            let options = PopupOptions {
                speed_search: true,
                min_width: Some(310.0),
                max_width: Some(500.0),
                ..PopupOptions::default()
            };
            ContextMenu::popup(rich_items(), options, window, cx)
        })
    }

    /// Titled separators, secondary texts, faded icons, row buttons, rows
    /// that run an action and open a submenu, rows shown only while
    /// filtering.
    #[gpui::test]
    fn rows_of_a_list_popup_with_extras(cx: &mut TestAppContext) {
        let (origin, menu, cx) = rich(cx);
        let state = menu.read_with(cx, |m, _| m.state_json());
        let items = &state["levels"][0]["items"];
        assert_eq!(items[0]["header"], json!("Pinned"));
        assert_eq!(items[1]["secondary"], json!("not supported yet"));
        let alpha = items[1]["icon_alpha"].as_f64().unwrap();
        assert!((alpha - 0.45).abs() < 1e-6, "{alpha}");
        assert_eq!(items[1]["command"], json!("menu_test::First"));
        assert_eq!(items[1]["inline"][0]["tooltip"], json!("Run 'alpha'"));
        assert_eq!(items[1]["inline"][1]["enabled"], json!(false));
        assert_eq!(items[4]["hidden"], json!(true));
        assert_eq!(shown_labels(&menu, cx), ["---", "alpha", "---", "beta"]);
        // The first row that can be chosen is selected; its buttons show.
        assert_eq!(selected(&menu, cx), [Some(1)]);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("menu-inline:Run 'alpha'").is_some());
        assert!(cx.debug_bounds("menu-header:Pinned").is_some());
        // Right opens its submenu; Enter on the row runs its own action.
        cx.dispatch_action(OpenSubmenu);
        assert_eq!(selected(&menu, cx), [Some(1), Some(0)]);
        cx.dispatch_action(CloseSubmenu);
        // A hidden row shows while filtering.
        cx.simulate_input("gam");
        assert_eq!(shown_labels(&menu, cx), ["gamma"]);
        cx.dispatch_action(Cancel);
        cx.dispatch_action(Confirm);
        cx.run_until_parked();
        assert_eq!(origin.read_with(cx, |o, _| o.got.clone()), ["first"]);
    }

    #[gpui::test]
    fn a_row_button_runs_its_action(cx: &mut TestAppContext) {
        let (origin, menu, cx) = rich(cx);
        cx.update(|window, cx| menu.update(cx, |m, cx| m.run_inline(0, 1, 0, window, cx)));
        cx.run_until_parked();
        assert_eq!(origin.read_with(cx, |o, _| o.got.clone()), ["second"]);
        assert!(menu.read_with(cx, |m, _| m.is_dismissed()));
    }
}
