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
//! commands. The menu emits [`DismissEvent`] when it closes and
//! [`HighlightEvent`] when another row is highlighted.
//!
//! Keyboard: Up and Down move through the innermost open level, Right and
//! Enter open a submenu, Left and Escape close it, Escape on the menu
//! itself closes the menu, Enter runs the selected item. A click outside
//! the menu closes it; so does deactivating the window. With speed search,
//! typing filters the rows of the popup (Backspace deletes, Escape clears
//! the filter before it closes the popup).
//!
//! Rows span the menu, with the shortcut of their action (from the active
//! keymap, where the focus was) at their end. In a context menu a submenu
//! opens while its row is selected; in a list popup it opens from the
//! row's arrow, its "More Actions" button or Right (`ListPopupImpl`
//! without `showSubmenuOnHover`). A submenu opens to the right of the
//! menu, or to its left when there is no room; the menu never moves.
//!
//! While the pointer moves from a row towards its open submenu, the rows it
//! crosses on the way do not take the hover (`MouseMovementTracker`,
//! `ListPopupImpl.isMovingToSubmenu`); a row the pointer rests on takes it
//! after a short delay. A submenu opened from a row's "More Actions" button
//! is pinned instead: the pointer moves the highlight over the other rows
//! but the submenu stays until the button is clicked again or the menu
//! closes (`ListPopupImpl` without `showSubmenuOnHover` keeps its child
//! open; `PopupInlineActionsSupportImpl.performAction`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Action, AnyElement, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement, Styled, StyledText, Subscription, Task, Window,
    anchored, deferred, div, point, prelude::*, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};

use crate::icons::{Icon, IconName};
use crate::pointer::PointerExt as _;
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
/// A row's buttons (`createExtraButton`): a 16 px icon with
/// `List.Button.leftRightInset` 8 on both sides, after a line of 1 with a
/// gap of 1 on both sides and `List.Button.separatorInset` 4 above and
/// below (`createButtonsSeparator`). The buttons, or a submenu's arrow, end
/// 5 inside the selection (`PopupListElementRenderer.customizeComponent`:
/// twice `Popup.Selection.leftRightInset` less the line); the arrow comes
/// 20 after the label (`myNextStepLabel`).
const BUTTON: f32 = 16.0 + 2.0 * 8.0;
const BUTTON_LINE: f32 = 3.0;
const BUTTON_LINE_INSET: f32 = 4.0;
const BUTTONS_RIGHT: f32 = 5.0;
const NEXT_STEP_GAP: f32 = 20.0;
/// Tooltip of the button that opens a row's submenu in a list popup
/// (`inline.actions.more.actions.text`).
pub const MORE_ACTIONS: &str = "More Actions";
/// The indent of a level of a row's label (`SchemesCombo`).
const INDENT: f32 = 10.0;
/// The gap before a secondary text (`emptyLeft(6)`) and before a shortcut.
const SECONDARY_GAP: f32 = 6.0;
const SHORTCUT_GAP: f32 = 18.0;
/// A titled separator (`GroupHeaderSeparator`): its line with
/// `Popup.separatorInsets` (4, 8), left out above the first row, then the
/// title in a smaller bold font (`JBFont.smallOrNewUiMedium`) with
/// `Popup.separatorLabelInsets` 3 above and below.
const HEADER_LABEL: f32 = 16.0;
const HEADER_INSET_Y: f32 = 3.0;
/// Distance a submenu keeps from the window's edges.
const WINDOW_MARGIN: f32 = 4.0;
/// Pointer positions kept to tell where it moves, and the distance that
/// counts as a move (`MouseMovementTracker`: 4 positions, 2 px).
const AIM_HISTORY: usize = 4;
const AIM_MARGIN: f32 = 2.0;
/// A row the pointer rests on while moving towards a submenu takes the
/// hover after this long (`ListPopupImpl`'s submenu timer, 250 ms).
const AIM_DELAY: Duration = Duration::from_millis(250);
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
    /// The shortcut at the end of the row; filled from the keymap when the
    /// menu opens (`KeymapUtil.getShortcutText`).
    pub shortcut: Option<SharedString>,
    /// A bold label (the predefined schemes of a schemes combo box).
    pub bold: bool,
    /// Levels the label is indented by (`SchemesCombo` indents a custom
    /// scheme under its parent).
    pub indent: u8,
}

impl Clone for RowExtra {
    fn clone(&self) -> Self {
        Self {
            secondary: self.secondary.clone(),
            icon_alpha: self.icon_alpha,
            inline: self.inline.clone(),
            hidden: self.hidden,
            shortcut: self.shortcut.clone(),
            bold: self.bold,
            indent: self.indent,
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

    /// A bold label.
    pub fn bold(mut self, bold: bool) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.bold = bold;
        }
        self
    }

    /// Indents the label by `levels`.
    pub fn indent(mut self, levels: u8) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.indent = levels;
        }
        self
    }

    /// The shortcut shown at the end of the row, instead of the one the
    /// keymap gives the row's action.
    pub fn shortcut(mut self, text: impl Into<SharedString>) -> Self {
        if let Some(extra) = self.extra_mut() {
            extra.shortcut = Some(text.into());
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

    /// Runs its own action and has a submenu (a "perform group", final in
    /// the reference's popup step).
    fn is_final_group(&self) -> bool {
        self.has_submenu() && self.run_action().is_some()
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
}

/// Fills the shortcuts of `items` (and their submenus) from the bindings
/// that win where `focus` is; a menu does it itself where it opens, a
/// popup in a window of its own gets them from the window its actions run
/// in.
pub fn fill_shortcuts(items: &mut [MenuItem], focus: &FocusHandle, window: &Window) {
    for item in items {
        match item {
            MenuItem::Action {
                action: Some(action),
                extra,
                ..
            } => {
                if extra.shortcut.is_none() {
                    extra.shortcut =
                        crate::keys::shortcut_in(action.as_ref(), focus, window).map(Into::into);
                }
            }
            MenuItem::Submenu { items, .. } => fill_shortcuts(items, focus, window),
            _ => {}
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
    /// The popup is the content of a window of its own
    /// ([`crate::dialog_window::open_popup`]): the window draws the shadow,
    /// and a chosen action is handed over as a [`ChosenEvent`] for the
    /// window it belongs to, instead of being dispatched in the popup's
    /// window.
    pub own_window: bool,
}

/// The action chosen in a popup that has a window of its own
/// ([`PopupOptions::own_window`]); the menu closes.
pub struct ChosenEvent(pub Box<dyn Action>);

/// The highlighted row changed (the keyboard, the pointer, the speed search,
/// a submenu that opened or closed): the action of the row now highlighted
/// in the innermost level that has one, `None` when that row runs no action
/// of its own (a submenu row) or nothing is highlighted. The reference's
/// list selection listener of a popup; its theme switcher previews the
/// highlighted theme with it (`QuickChangeLookAndFeel`).
pub struct HighlightEvent(pub Option<Box<dyn Action>>);

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
    own_window: bool,
    /// A submenu opens while its row is selected (context menus), else
    /// from its arrow, its button or Right (list popups).
    submenu_on_hover: bool,
    /// Where each open submenu is drawn (window coordinates), by level.
    submenu_bounds: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
    /// The pointer's last positions over the menu, newest at `aim_at`.
    aim_history: [Option<Point<Pixels>>; AIM_HISTORY],
    aim_at: usize,
    /// A row the pointer reached while moving towards a submenu, where it
    /// was, and the timer that gives it the hover if the pointer rests.
    aim_pending: Option<(usize, usize, Point<Pixels>)>,
    _aim_timer: Option<Task<()>>,
    /// The level whose submenu was opened by a row's More Actions button,
    /// and the row of that level the pointer is over meanwhile (it shows
    /// the highlight and the buttons; the submenu stays with its row).
    pinned: Option<usize>,
    pinned_hover: Option<usize>,
    /// The highlighted row last told with a [`HighlightEvent`]: the rows
    /// selected down to the innermost one.
    highlighted: Vec<usize>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ContextMenu {}
impl EventEmitter<ChosenEvent> for ContextMenu {}
impl EventEmitter<HighlightEvent> for ContextMenu {}

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
        let mut items = items;
        if let Some(origin) = &origin {
            fill_shortcuts(&mut items, origin, window);
        }
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
                own_window: options.own_window,
                submenu_on_hover: !list,
                submenu_bounds: Rc::default(),
                aim_history: [None; AIM_HISTORY],
                aim_at: 0,
                aim_pending: None,
                _aim_timer: None,
                pinned: None,
                pinned_hover: None,
                highlighted: Vec::new(),
                _subscriptions: subscriptions,
            };
            if list {
                menu.selected[0] = options
                    .selected
                    .filter(|&ix| menu.items.get(ix).is_some_and(MenuItem::selectable))
                    .or_else(|| menu.first_selectable());
            }
            menu.highlighted = menu.highlight_path();
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
        self.highlight_changed(cx);
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

    /// Selects row `ix` of `level`, closing deeper levels; with `open` a
    /// submenu row opens its submenu (with nothing selected yet).
    fn select(&mut self, level: usize, ix: usize, open: bool, cx: &mut Context<Self>) {
        let Some(item) = self.level_items(level).get(ix) else {
            return;
        };
        if !item.selectable() {
            return;
        }
        let submenu = open && item.has_submenu();
        self.selected.truncate(level + 1);
        self.selected[level] = Some(ix);
        if submenu {
            self.selected.push(None);
        }
        self.unpin_closed();
        self.highlight_changed(cx);
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
                self.highlight_changed(cx);
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
        self.select(level, ix, true, cx);
        self.step(1, cx);
        true
    }

    /// The pointer moved to `position` over row `ix` of `level`: the row
    /// takes the hover unless the pointer moves towards the open submenu
    /// of that level; then it does after [`AIM_DELAY`] if the pointer
    /// rests there.
    fn hover(&mut self, level: usize, ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        // A pinned submenu stays; the pointer only moves the highlight.
        if self.pinned == Some(level) {
            if self.pinned_hover != Some(ix) {
                self.pinned_hover = Some(ix);
                cx.notify();
            }
            return;
        }
        let towards = self.moving_towards_submenu(level, position);
        let hovered =
            self.selected.get(level).copied().flatten() == Some(ix) && self.selected.len() > level;
        if hovered {
            self.aim_pending = None;
            return;
        }
        if towards {
            self.aim_pending = Some((level, ix, position));
            self._aim_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AIM_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some((level, ix, at)) = this.aim_pending.take()
                        && this.aim_history[this.aim_at] == Some(at)
                    {
                        let open = this.submenu_on_hover;
                        this.select(level, ix, open, cx);
                    }
                });
            }));
            return;
        }
        self.aim_pending = None;
        let open = self.submenu_on_hover;
        self.select(level, ix, open, cx);
    }

    /// Keeps `position` in the history and tells whether the pointer moves
    /// towards the submenu open from `level` (`MouseMovementTracker.
    /// isMovingTowards`, `ScreenUtil.isMovementTowards`).
    fn moving_towards_submenu(&mut self, level: usize, position: Point<Pixels>) -> bool {
        let far_enough = |a: Point<Pixels>, b: Point<Pixels>| {
            let dx = f32::from(a.x - b.x);
            let dy = f32::from(a.y - b.y);
            (dx * dx + dy * dy).sqrt() >= AIM_MARGIN
        };
        let previous = (0..AIM_HISTORY)
            .filter_map(|i| self.aim_history[(self.aim_at + AIM_HISTORY - i) % AIM_HISTORY])
            .find(|p| far_enough(*p, position));
        if self.aim_history[self.aim_at].is_none_or(|p| far_enough(p, position)) {
            self.aim_at = (self.aim_at + 1) % AIM_HISTORY;
            self.aim_history[self.aim_at] = Some(position);
        }
        if self.selected.len() <= level + 1 {
            return false;
        }
        let bounds = self
            .submenu_bounds
            .borrow()
            .get(level + 1)
            .copied()
            .flatten();
        let Some(bounds) = bounds else {
            return false;
        };
        movement_towards(previous, position, bounds)
    }

    /// The "More Actions" button of row `ix`: opens its submenu with the
    /// first row selected (`PopupInlineActionsSupportImpl.performAction`).
    /// The button again closes the submenu; another row's button moves it
    /// there. The submenu is pinned (see [`ContextMenu::hover`]).
    fn open_more(&mut self, level: usize, ix: usize, cx: &mut Context<Self>) {
        if level >= self.selected.len() {
            return;
        }
        let open_here = self.selected.len() > level + 1 && self.selected[level] == Some(ix);
        self.selected.truncate(level + 1);
        self.selected[level] = Some(ix);
        self.pinned_hover = None;
        if open_here {
            self.pinned = None;
            self.highlight_changed(cx);
            cx.notify();
            return;
        }
        if self.open_selected_submenu(cx) {
            self.pinned = Some(level);
        }
        self.highlight_changed(cx);
    }

    /// Forgets a pin whose submenu is closed.
    fn unpin_closed(&mut self) {
        if self
            .pinned
            .is_some_and(|level| self.selected.len() <= level + 1)
        {
            self.pinned = None;
            self.pinned_hover = None;
        }
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
            self.unpin_closed();
            self.highlight_changed(cx);
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.len() > 1 {
            self.selected.pop();
            self.unpin_closed();
            self.highlight_changed(cx);
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
            self.choose(action, window, cx);
            return;
        }
        match item {
            MenuItem::Submenu { .. }
                if self.selected.get(level + 1).is_none() || self.selected[level] != Some(ix) =>
            {
                self.select(level, ix, true, cx);
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
        self.choose(action, window, cx);
    }

    /// Closes the menu and runs `action` where the focus was, or hands it
    /// over when the menu has a window of its own.
    fn choose(&mut self, action: Box<dyn Action>, window: &mut Window, cx: &mut Context<Self>) {
        if self.own_window {
            cx.emit(ChosenEvent(action));
            self.dismiss(window, cx);
            return;
        }
        self.dismiss(window, cx);
        // The focus is back where it was; the action goes there.
        window.dispatch_action(action, cx);
    }

    /// The rows selected down to the innermost highlighted one.
    fn highlight_path(&self) -> Vec<usize> {
        let end = self
            .selected
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |i| i + 1);
        self.selected[..end]
            .iter()
            .map(|ix| ix.unwrap_or(0))
            .collect()
    }

    /// The action of the highlighted row (see [`HighlightEvent`]).
    pub fn highlighted_action(&self) -> Option<&dyn Action> {
        let level = self.selected.iter().rposition(Option::is_some)?;
        let ix = self.selected[level]?;
        self.level_items(level).get(ix)?.run_action()
    }

    /// Emits a [`HighlightEvent`] when another row is highlighted.
    fn highlight_changed(&mut self, cx: &mut Context<Self>) {
        let path = self.highlight_path();
        if path != self.highlighted {
            self.highlighted = path;
            let action = self.highlighted_action().map(|a| a.boxed_clone());
            cx.emit(HighlightEvent(action));
        }
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
                            if extra.bold {
                                row["bold"] = json!(true);
                            }
                            if extra.indent > 0 {
                                row["indent"] = json!(extra.indent);
                            }
                            if let Some(text) = &extra.secondary {
                                row["secondary"] = json!(text.as_ref());
                            }
                            if let Some(alpha) = extra.icon_alpha {
                                row["icon_alpha"] = json!(alpha);
                            }
                            if let Some(shortcut) = &extra.shortcut {
                                row["shortcut"] = json!(shortcut.as_ref());
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
            "pinned": self.pinned,
        })
    }

    // ---- drawing ------------------------------------------------------------

    fn render_level(&self, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let m = self.metrics;
        let list = m == LIST;
        let items = self.level_items(level);
        let shown = self.shown(level);
        let with_icons = items.iter().any(|i| i.icon_name().is_some());
        let selected = self.selected[level];
        let open = self.selected.len() > level + 1;
        let selection = theme.color(UiRole::Selection);
        let text = theme.color(UiRole::Text);
        let muted = theme.color(UiRole::TextMuted);
        let disabled = theme.color(UiRole::TextDisabled);
        let line = theme.color(UiRole::PopupBorder);
        let match_bg = theme.color(UiRole::SearchMatch);
        let small = theme.ui_size() - px(1.0);
        let item_height = self.item_height(cx);
        let inner_height = item_height - 2.0 * m.selection_y;
        let first_shown = shown.iter().position(|s| *s);
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter(|(ix, _)| shown[*ix])
            .map(|(ix, item)| {
                let id = SharedString::from(format!("menu-{level}-{ix}"));
                if let MenuItem::Header(title) = item {
                    let selector = format!("menu-header:{title}");
                    let first = first_shown == Some(ix);
                    // Columns, so that the line stretches across.
                    return div()
                        .id(id)
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_col()
                        .px(px(m.separator_x))
                        .when(!first, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .h(px(1.0))
                                    .my(px(SEPARATOR_INDENT))
                                    .bg(line),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .h(px(HEADER_LABEL))
                                .my(px(HEADER_INSET_Y))
                                .px(px(m.content_x))
                                .text_size(small)
                                .font_weight(FontWeight::BOLD)
                                .text_color(muted)
                                .whitespace_nowrap()
                                .child(title.clone()),
                        )
                        .into_any_element();
                }
                if let MenuItem::Separator = item {
                    let selector = format!("menu-separator-{level}-{ix}");
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
                // The row whose submenu is open, and the highlighted row:
                // the same but under a pinned submenu, where the pointer
                // moves the highlight alone.
                let owner = selected == Some(ix);
                let is_selected = match (self.pinned == Some(level), self.pinned_hover) {
                    (true, Some(hovered)) => hovered == ix,
                    _ => owner,
                };
                let label = item.label().cloned().unwrap_or_default();
                let selector = format!("menu-item:{label}");
                let inner_selector = format!("menu-row:{label}");
                let extra = item.extra();
                let secondary = extra.and_then(|e| e.secondary.clone());
                let shortcut = extra.and_then(|e| e.shortcut.clone());
                let icon_alpha = extra.and_then(|e| e.icon_alpha).unwrap_or(1.0);
                let bold = extra.is_some_and(|e| e.bold);
                let indent = extra.map_or(0, |e| e.indent);
                let more = !self.submenu_on_hover && item.is_final_group();
                let buttons = extra.map_or(0, |e| e.inline.len()) + usize::from(more);
                let arrow = item.has_submenu() && !more;
                let shortcut_color = if enabled { muted } else { disabled };
                let shortcut = shortcut.map(|s| {
                    div()
                        .flex_none()
                        .pl(px(SHORTCUT_GAP))
                        .whitespace_nowrap()
                        .text_color(shortcut_color)
                        .child(s)
                        .into_any_element()
                });
                // The row's end: its buttons (their room while it is not
                // selected), a submenu's arrow or the shortcut.
                let end: Option<AnyElement> = if buttons > 0 {
                    Some(self.render_buttons(level, ix, more, is_selected, inner_height, cx))
                } else if list && arrow {
                    Some(
                        div()
                            .flex_none()
                            .pl(px(NEXT_STEP_GAP))
                            .child(Icon::new(IconName::CHEVRON_RIGHT))
                            .into_any_element(),
                    )
                } else if list {
                    shortcut
                } else {
                    // A popup menu keeps room for the arrow after every
                    // label; a shortcut or an arrow ends at the row's end.
                    Some(
                        div()
                            .flex_none()
                            .min_w(px(TRAILING))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_end()
                            .children(shortcut)
                            .when(arrow, |d| d.child(Icon::new(IconName::CHEVRON_RIGHT)))
                            .into_any_element(),
                    )
                };
                let right = if list && (buttons > 0 || arrow) {
                    BUTTONS_RIGHT
                } else {
                    m.content_x
                };
                let submenu = (open && owner).then(|| Beside {
                    child: self.render_level(level + 1, cx),
                    above: px(1.0 + m.pad_top),
                    level: level + 1,
                    drawn: self.submenu_bounds.clone(),
                });
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
                    // A row, so that the selection fills it whatever the
                    // label's width (#128).
                    .flex()
                    .flex_row()
                    .child(
                        div()
                            .debug_selector(move || inner_selector.clone())
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .pl(px(m.content_x))
                            .pr(px(right))
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
                                    .child(
                                        div()
                                            .flex_none()
                                            .whitespace_nowrap()
                                            .pl(px(INDENT * f32::from(indent)))
                                            .when(bold, |d| d.font_weight(FontWeight::BOLD))
                                            .child(label),
                                    )
                                    .children(secondary.map(|text| {
                                        div()
                                            .flex_none()
                                            .pl(px(SECONDARY_GAP))
                                            .whitespace_nowrap()
                                            .text_color(muted)
                                            .child(text)
                                    })),
                            )
                            .children(end),
                    )
                    .children(submenu)
                    .when(enabled, |d| {
                        d.on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            this.hover(level, ix, event.position, cx);
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
        let theme = cx.theme();
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
            .when(!self.own_window, |d| {
                d.shadow(crate::components::popup_shadow())
            })
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
                    .children(rows)
                    // The hand over a list popup's list (`ListPopupImpl`);
                    // the rows of a context menu keep the arrow.
                    .when(list, |d| d.pointing_hand()),
            )
            .into_any_element()
    }

    /// The buttons at the end of row `ix` and the line before them
    /// (`PopupListElementRenderer.updateExtraButtons`), the last one opening
    /// its submenu when `more`. While the row is not selected only their
    /// room, so that nothing moves when they show.
    fn render_buttons(
        &self,
        level: usize,
        ix: usize,
        more: bool,
        show: bool,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let hover = theme.color(UiRole::ListButtonHover);
        let line = theme.color(UiRole::ListButtonSeparator);
        let inline: Vec<InlineButton> = self
            .level_items(level)
            .get(ix)
            .and_then(MenuItem::extra)
            .map(|e| e.inline.clone())
            .unwrap_or_default();
        let count = inline.len() + usize::from(more);
        if !show {
            return div()
                .flex_none()
                .w(px(BUTTON_LINE + count as f32 * BUTTON))
                .into_any_element();
        }
        let button = |id: SharedString, selector: String, icon: IconName, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || selector.clone())
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .w(px(BUTTON))
                .h(px(height))
                .rounded(px(SELECTION_RADIUS))
                .when(enabled, |d| d.hover(|s| s.bg(hover)))
                .when(!enabled, |d| d.opacity(crate::components::DISABLED_ALPHA))
                .child(Icon::new(icon))
        };
        let mut buttons: Vec<AnyElement> = inline
            .into_iter()
            .enumerate()
            .map(|(b, inline)| {
                let enabled = inline.action.is_some();
                let tooltip = inline.tooltip.clone();
                button(
                    SharedString::from(format!("menu-inline-{level}-{ix}-{b}")),
                    format!("menu-inline:{tooltip}"),
                    inline.icon,
                    enabled,
                )
                .tooltip(crate::components::Tooltip::text(tooltip, None))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.run_inline(level, ix, b, window, cx);
                    }))
                })
                .into_any_element()
            })
            .collect();
        if more {
            buttons.push(
                button(
                    SharedString::from(format!("menu-more-{level}-{ix}")),
                    format!("menu-inline:{MORE_ACTIONS}"),
                    IconName::MORE_VERTICAL,
                    true,
                )
                .tooltip(crate::components::Tooltip::text(MORE_ACTIONS, None))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_more(level, ix, cx);
                }))
                .into_any_element(),
            );
        }
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .w(px(BUTTON_LINE))
                    .h(px(height))
                    .py(px(BUTTON_LINE_INSET))
                    .child(div().w(px(1.0)).bg(line)),
            )
            .children(buttons)
            .into_any_element()
    }

    /// Height of an item row: a list row of the theme, and in popup menus
    /// the selection's outer insets.
    fn item_height(&self, cx: &App) -> f32 {
        f32::from(cx.theme().metric(Metric::RowHeight)) + 2.0 * self.metrics.selection_y
    }
}

/// A submenu beside its row (`WizardPopup.show`): right of the menu, or
/// left of it when there is no room on the right, moved up to stay inside
/// the window, with its first row level with its parent row. It is laid
/// out on its own, so opening it neither widens nor moves the menu.
struct Beside {
    child: AnyElement,
    /// From the top of the submenu to its first row: the border and the
    /// space above the rows.
    above: Pixels,
    /// The submenu's level, and where the menu keeps the bounds it is
    /// drawn at (for the pointer's aim).
    level: usize,
    drawn: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
}

impl IntoElement for Beside {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for Beside {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        // Over its row, out of the flow: its bounds are the row's.
        let style = gpui::Style {
            position: gpui::Position::Absolute,
            inset: gpui::Edges::all(px(0.0).into()),
            ..gpui::Style::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        row: gpui::Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = self.child.layout_as_root(
            gpui::size(
                gpui::AvailableSpace::MaxContent,
                gpui::AvailableSpace::MaxContent,
            ),
            window,
            cx,
        );
        let viewport = window.viewport_size();
        let margin = px(WINDOW_MARGIN);
        // The menu's border is 1 outside its rows.
        let border = px(1.0);
        let mut x = row.right() + border;
        if x + size.width > viewport.width - margin {
            let left = row.left() - border - size.width;
            x = if left >= margin {
                left
            } else {
                (viewport.width - margin - size.width).max(margin)
            };
        }
        let y = (row.top() - self.above)
            .min(viewport.height - margin - size.height)
            .max(margin);
        {
            let mut drawn = self.drawn.borrow_mut();
            if drawn.len() <= self.level {
                drawn.resize(self.level + 1, None);
            }
            drawn[self.level] = Some(Bounds::new(point(x, y), size));
        }
        self.child.prepaint_at(point(x, y), window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// The pointer moves from `previous` to `location` towards `bounds`: the
/// line through both points crosses the rectangle ahead of the pointer
/// (`ScreenUtil.isMovementTowards`). Without a previous position, or
/// inside the rectangle, it does.
fn movement_towards(
    previous: Option<Point<Pixels>>,
    location: Point<Pixels>,
    bounds: Bounds<Pixels>,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    if previous == location || bounds.contains(&location) {
        return true;
    }
    let (px_, py) = (f32::from(previous.x), f32::from(previous.y));
    let (lx, ly) = (f32::from(location.x), f32::from(location.y));
    let (bx, by) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let dx = px_ - lx;
    let dy = py - ly;
    // Moving away from the rectangle.
    if dx > 0.0 && bx >= px_ {
        return false;
    }
    if dx < 0.0 && bx + bw <= px_ {
        return false;
    }
    if dy < 0.0 && by + bh <= py {
        return false;
    }
    if dy > 0.0 && by >= py {
        return false;
    }
    if dx == 0.0 {
        return lx >= bx && lx < bx + bw && ((dy > 0.0) ^ (by > ly));
    }
    if dy == 0.0 {
        return ly >= by && ly < by + bh && ((dx > 0.0) ^ (bx > lx));
    }
    // The line `y = a x + b` through both points crosses a side.
    let a = dy / dx;
    let b = ly - a * lx;
    let within_y = |y: f32| y >= by && y < by + bh;
    let within_x = |x: f32| x >= bx && x < bx + bw;
    within_y(a * bx + b)
        || within_y(a * (bx + bw) + b)
        || within_x((by - b) / a)
        || within_x((by + bh - b) / a)
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
        // The menu; each open submenu is drawn beside its row.
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
            .child(self.render_level(0, cx))
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
        /// Draws the menu as the application does, with [`overlay`] at
        /// this point.
        at: Option<Point<Pixels>>,
    }

    impl Render for Origin {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let menu = self.menu.clone().map(|menu| match self.at {
                Some(at) => overlay(&menu, at, window),
                None => menu.into_any_element(),
            });
            div()
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &First, _, _| this.got.push("first")))
                .on_action(cx.listener(|this, _: &Second, _, _| this.got.push("second")))
                .on_action(cx.listener(|this, _: &Third, _, _| this.got.push("third")))
                .size_full()
                .child(crate::pointer::frame())
                .children(menu)
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
            at: None,
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

    /// Draws the menu with [`overlay`] at `at`, as the application does.
    fn place(origin: &Entity<Origin>, at: Point<Pixels>, cx: &mut VisualTestContext) {
        origin.update(cx, |o, cx| {
            o.at = Some(at);
            cx.notify();
        });
        redraw(cx);
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
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
    fn moving_the_highlight_tells_the_highlighted_rows_action(cx: &mut TestAppContext) {
        let (_, menu, cx) = open(cx);
        let seen: Rc<std::cell::RefCell<Vec<Option<&'static str>>>> = Rc::default();
        let log = seen.clone();
        cx.update(|_, cx| {
            cx.subscribe(&menu, move |_, e: &HighlightEvent, _| {
                log.borrow_mut().push(e.0.as_ref().map(|a| a.name()))
            })
            .detach()
        });
        fn step(action: impl Action, cx: &mut VisualTestContext) {
            cx.dispatch_action(action);
            cx.run_until_parked();
        }
        step(SelectNext, cx);
        step(SelectNext, cx);
        // Into the submenu and out again: its row, then the submenu row.
        step(OpenSubmenu, cx);
        step(CloseSubmenu, cx);
        let name = |a: &dyn Action| a.name();
        assert_eq!(
            *seen.borrow(),
            [Some(name(&Second)), None, Some(name(&First)), None]
        );
        // The same row again tells nothing.
        step(OpenSubmenu, cx);
        step(SelectNext, cx);
        assert_eq!(seen.borrow().len(), 5);
        assert_eq!(
            menu.read_with(cx, |m, _| m.highlighted_action().map(|a| a.name())),
            Some(name(&First))
        );
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

    /// The pointer shape over the element with this debug selector.
    fn shape_over(selector: &'static str, cx: &mut VisualTestContext) -> gpui::CursorStyle {
        let at = cx
            .debug_bounds(selector)
            .expect("the row is drawn")
            .center();
        cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        redraw(cx);
        cx.update(|window, _| crate::pointer::resolve(window))
    }

    #[gpui::test]
    fn a_list_popup_shows_the_hand_over_its_rows_and_a_context_menu_the_arrow(
        cx: &mut TestAppContext,
    ) {
        // `ListPopupImpl` sets the hand on its list; a popup menu is not one.
        let (_, _, popup_cx) = popup(cx, None);
        assert_eq!(
            shape_over("menu-item:uv (demo)", popup_cx),
            gpui::CursorStyle::PointingHand
        );
        let (_, _, menu_cx) = open(cx);
        assert_eq!(
            shape_over("menu-item:Second", menu_cx),
            gpui::CursorStyle::Arrow
        );
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

    /// Every row spans the menu's width, whatever its text: the selection
    /// background goes across the menu (#128).
    #[gpui::test]
    fn rows_span_the_menu(cx: &mut TestAppContext) {
        let (origin, _, cx) = open_with(cx, |window, cx| {
            ContextMenu::build(
                vec![
                    MenuItem::action("A", First),
                    MenuItem::action("A much longer row of the menu", Second),
                    MenuItem::submenu("More", vec![MenuItem::action("x", Third)]),
                ],
                window,
                cx,
            )
        });
        place(&origin, point(px(10.0), px(10.0)), cx);
        let level = cx.debug_bounds("menu-level-0").unwrap();
        for (name, inner) in [
            ("menu-item:A", "menu-row:A"),
            (
                "menu-item:A much longer row of the menu",
                "menu-row:A much longer row of the menu",
            ),
            ("menu-item:More", "menu-row:More"),
        ] {
            let row = cx.debug_bounds(name).unwrap();
            assert_eq!(row.size.width, level.size.width - px(2.0), "{name}");
            let inner = cx.debug_bounds(inner).unwrap();
            assert_eq!(
                inner.size.width,
                row.size.width - px(2.0 * MENU.selection_x),
                "{name}"
            );
        }
    }

    /// A row shows the shortcut its action has where the focus was (#69).
    #[gpui::test]
    fn rows_show_the_shortcut_of_their_action(cx: &mut TestAppContext) {
        cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("cmd-k", Second, None)]));
        let (_, menu, cx) = open(cx);
        let expected = crate::keys::binding_text(&gpui::KeyBinding::new("cmd-k", Second, None));
        let state = menu.read_with(cx, |m, _| m.state_json());
        let items = &state["levels"][0]["items"];
        assert_eq!(items[2]["label"], json!("Second"));
        assert_eq!(items[2]["shortcut"], json!(expected));
        assert_eq!(items[0].get("shortcut"), None, "no binding, no shortcut");
    }

    /// In a list popup a row's submenu opens from its "More Actions"
    /// button (or Right), not on hover; the buttons show on the selected
    /// row, in room every such row keeps, so nothing moves (#127).
    #[gpui::test]
    fn a_list_popup_opens_a_submenu_from_its_button(cx: &mut TestAppContext) {
        let (origin, menu, cx) = rich(cx);
        place(&origin, point(px(10.0), px(10.0)), cx);
        let level = cx.debug_bounds("menu-level-0").unwrap();
        let alpha = cx.debug_bounds("menu-item:alpha").unwrap();
        let beta = cx.debug_bounds("menu-item:beta").unwrap();
        cx.simulate_mouse_move(alpha.center(), None, gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(1)], "hovering opens nothing");
        let row = cx.debug_bounds("menu-row:alpha").unwrap();
        let more = cx.debug_bounds("menu-inline:More Actions").unwrap();
        assert_eq!(more.right(), row.right() - px(BUTTONS_RIGHT));
        assert_eq!(more.size.width, px(BUTTON));
        cx.simulate_mouse_move(beta.center(), None, gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(3)]);
        assert_eq!(
            cx.debug_bounds("menu-level-0"),
            Some(level),
            "nothing moves"
        );
        cx.simulate_mouse_move(alpha.center(), None, gpui::Modifiers::none());
        redraw(cx);
        let more = cx.debug_bounds("menu-inline:More Actions").unwrap();
        cx.simulate_click(more.center(), gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(1), Some(0)]);
        assert_eq!(cx.debug_bounds("menu-level-0"), Some(level));
        let sub = cx.debug_bounds("menu-level-1").unwrap();
        assert_eq!(sub.left(), level.right());
        assert!(origin.read_with(cx, |o, _| o.got.is_empty()));
        assert!(!menu.read_with(cx, |m, _| m.is_dismissed()));
    }

    /// A submenu opens right of the menu, level with its row, or left of it
    /// when there is no room on the right; the menu never moves (#127).
    #[gpui::test]
    fn a_submenu_without_room_on_the_right_opens_on_the_left(cx: &mut TestAppContext) {
        let (origin, menu, cx) = open(cx);
        let viewport = cx.update(|window, _| window.viewport_size());
        place(&origin, point(viewport.width - px(20.0), px(10.0)), cx);
        let level = cx.debug_bounds("menu-level-0").unwrap();
        assert!(level.right() <= viewport.width);
        cx.dispatch_action(SelectNext);
        cx.dispatch_action(SelectNext);
        cx.dispatch_action(OpenSubmenu);
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(3), Some(0)]);
        assert_eq!(
            cx.debug_bounds("menu-level-0"),
            Some(level),
            "the menu stays"
        );
        let sub = cx.debug_bounds("menu-level-1").unwrap();
        assert_eq!(sub.right(), level.left());
        let row = cx.debug_bounds("menu-item:More").unwrap();
        assert_eq!(sub.top(), row.top() - px(1.0 + MENU.pad_top));
        // With room on the right it opens there.
        place(&origin, point(px(10.0), px(10.0)), cx);
        let level = cx.debug_bounds("menu-level-0").unwrap();
        let sub = cx.debug_bounds("menu-level-1").unwrap();
        assert_eq!(sub.left(), level.right());
    }

    fn aim_items() -> Vec<MenuItem> {
        vec![
            MenuItem::action("Second", Second),
            MenuItem::submenu(
                "More",
                vec![
                    MenuItem::action("One", First),
                    MenuItem::action("Two", First),
                    MenuItem::action("Three", First),
                    MenuItem::action("Four", First),
                    MenuItem::action("Five", First),
                ],
            ),
            MenuItem::action("Third", Third),
            MenuItem::action("Fourth", Third),
        ]
    }

    fn move_to(cx: &mut VisualTestContext, at: Point<Pixels>) {
        cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        redraw(cx);
    }

    /// The safety triangle (#135): while the pointer moves from a row
    /// towards its submenu, the rows it crosses do not take the hover; a
    /// row it rests on does after a moment; a move away from the submenu
    /// changes the hover at once.
    #[gpui::test]
    fn rows_on_the_way_to_a_submenu_keep_it_open(cx: &mut TestAppContext) {
        let (origin, menu, cx) =
            open_with(cx, |window, cx| ContextMenu::build(aim_items(), window, cx));
        place(&origin, point(px(10.0), px(10.0)), cx);
        let level = cx.debug_bounds("menu-level-0").unwrap();
        let more = cx.debug_bounds("menu-item:More").unwrap();
        let third = cx.debug_bounds("menu-item:Third").unwrap();
        // Hovering More opens its submenu to the right.
        let start = point(level.right() - px(30.0), more.center().y);
        move_to(cx, start);
        assert_eq!(selected(&menu, cx), [Some(1), None]);
        let sub = cx.debug_bounds("menu-level-1").unwrap();
        assert!(
            sub.bottom() > third.bottom(),
            "the submenu reaches below Third"
        );
        // Down and right, towards the submenu, over Third: More keeps it.
        let on_third = point(level.right() - px(10.0), third.center().y);
        move_to(cx, on_third);
        assert_eq!(selected(&menu, cx), [Some(1), None]);
        // Resting there gives Third the hover.
        cx.executor()
            .advance_clock(AIM_DELAY + Duration::from_millis(50));
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(2)]);
        // Straight down, away from the submenu: the row takes the hover.
        move_to(cx, start);
        assert_eq!(selected(&menu, cx), [Some(1), None]);
        let fourth = cx.debug_bounds("menu-item:Fourth").unwrap();
        move_to(cx, point(start.x, fourth.center().y));
        assert_eq!(selected(&menu, cx), [Some(3)]);
    }

    #[test]
    fn movement_towards_a_rectangle() {
        let b = Bounds::new(point(px(100.0), px(0.0)), gpui::size(px(50.0), px(100.0)));
        let p = |x: f32, y: f32| point(px(x), px(y));
        // Right and a little down: crosses the left side.
        assert!(movement_towards(Some(p(50.0, 10.0)), p(60.0, 15.0), b));
        // Steeply down: passes below.
        assert!(!movement_towards(Some(p(50.0, 10.0)), p(55.0, 40.0), b));
        // Left, away from it.
        assert!(!movement_towards(Some(p(60.0, 10.0)), p(50.0, 10.0), b));
        // Straight down beside it.
        assert!(!movement_towards(Some(p(50.0, 10.0)), p(50.0, 20.0), b));
        // Inside, or without a previous position.
        assert!(movement_towards(Some(p(50.0, 10.0)), p(120.0, 10.0), b));
        assert!(movement_towards(None, p(0.0, 0.0), b));
    }

    /// A submenu opened from a row's More Actions button stays while the
    /// pointer moves over the other rows; the button again closes it
    /// (#135, `ListPopupImpl` without `showSubmenuOnHover`).
    #[gpui::test]
    fn a_submenu_from_its_button_is_pinned(cx: &mut TestAppContext) {
        let (origin, menu, cx) = rich(cx);
        place(&origin, point(px(10.0), px(10.0)), cx);
        let alpha = cx.debug_bounds("menu-item:alpha").unwrap();
        let beta = cx.debug_bounds("menu-item:beta").unwrap();
        let more = cx.debug_bounds("menu-inline:More Actions").unwrap();
        cx.simulate_click(more.center(), gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(1), Some(0)]);
        // Over beta, and back: the submenu stays with alpha.
        move_to(cx, beta.center());
        assert_eq!(selected(&menu, cx), [Some(1), Some(0)]);
        let state = menu.read_with(cx, |m, _| m.state_json());
        assert_eq!(state["pinned"], json!(0));
        move_to(cx, point(beta.center().x, beta.center().y + px(2.0)));
        assert_eq!(selected(&menu, cx), [Some(1), Some(0)]);
        move_to(cx, alpha.center());
        // The button again closes it.
        let more = cx.debug_bounds("menu-inline:More Actions").unwrap();
        cx.simulate_click(more.center(), gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(selected(&menu, cx), [Some(1)]);
        let state = menu.read_with(cx, |m, _| m.state_json());
        assert_eq!(state["pinned"], Json::Null);
        // Open again; a click on another row runs it.
        cx.simulate_click(more.center(), gpui::Modifiers::none());
        redraw(cx);
        move_to(cx, beta.center());
        cx.simulate_click(beta.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(origin.read_with(cx, |o, _| o.got.clone()), ["second"]);
    }
}
