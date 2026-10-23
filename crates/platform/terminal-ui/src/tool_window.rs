//! The Terminal tool window (the reference's reworked terminal,
//! `TerminalToolWindowManager`, `TerminalTabsManager`): a header with the
//! title, one tab per session (its title and ×, the selected tab outlined,
//! "+" and the chevron after them; ⋮ and "−" on the right) and the selected
//! session below.
//!
//! The header is drawn as the reference's New UI with islands draws a tool
//! window's header (`ToolWindowHeader`, `TabContentLayout`,
//! `IslandsTabPainter`): 40 high with a line under it, the tabs as pills
//! that are outlined in the focus colours while the tool window is active
//! and in grey otherwise; the buttons show only while the tool window is
//! active, has a popup open or is under the pointer
//! (`InternalDecoratorImpl.updateActiveAndHoverState`).
//!
//! A tab's title is the name the user gave it, else the title the running
//! program set (cut to 30 characters in the middle) while a command runs,
//! else its default name (`TerminalTitle.buildTitle`,
//! `TerminalTitleUpdating`).
//!
//! The window does not start sessions itself: "+" and New Tab ask the owner
//! ([`TerminalToolWindowEvent::NewTab`]), which plans the session with the
//! product's customizers and hands it over with [`TerminalToolWindow::add_tab`].

use std::cell::Cell;
use std::rc::Rc;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Corner, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, PathBuilder, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, canvas, div,
    point, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use themes::UiRole;
use ui::components::{HEADER_TAB_HEIGHT, Tooltip, header_tab, icon_button};
use ui::deferred;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use crate::commands::{
    CloseAllTabs, CloseTab, GroupTabs, NewTab, NextTab, PreviousTab, RemoveFromSidebar,
    RenameCancel, RenameConfirm, RenameTab, SelectEngine, SelectTab, ShowOptions, ShowSettings,
    ShowTabList,
};
use crate::{SessionView, SessionViewEvent};

/// The key context of the tool window (its header and sessions).
pub const KEY_CONTEXT: &str = "TerminalToolWindow";
/// The key context of the rename balloon.
pub const RENAME_CONTEXT: &str = "TerminalRename";
/// The default name of a tab (`TerminalOptionsProvider.tabName`).
pub const DEFAULT_TAB_NAME: &str = "Local";
/// A program's title is cut to this many characters in a tab
/// (`TerminalTitle.shortenApplicationTitle`).
pub const TITLE_LIMIT: usize = 30;

const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;
/// The header's height above its line (`ToolWindowHeader`: 41 with it).
const HEADER_HEIGHT: f32 = 40.0;
/// Around the title (`ToolWindow.Header.labelLeftRightInsets`: 12, 16).
const TITLE_LEFT: f32 = 12.0;
const TITLE_RIGHT: f32 = 16.0;
/// A tab's pill sits 4 inside its label's bounds on the left
/// (`IslandsTabPainter`'s horizontal offset) and 5 on the right, its text
/// 7 inside the pill: 12 from the label's left
/// (`ToolWindow.HeaderTab.leftRightInsets`; measured on the reference).
const TAB_OFFSET: f32 = 4.0;
const TAB_OFFSET_RIGHT: f32 = 5.0;
const TAB_TEXT: f32 = 7.0;
/// Between a tab's text and its close button, and after the button.
const TAB_CLOSE_GAP: f32 = 8.0;
const TAB_CLOSE_RIGHT: f32 = 1.5;
/// The list of tabs opens under the header, its border 9 left of the
/// selected tab's pill (the popup under the tab's label, in its window
/// with its shadow), at least 108 wide (measured on the reference).
const TAB_LIST_LEFT: f32 = -9.0;
const TAB_LIST_MIN_WIDTH: f32 = 108.0;
/// The rename balloon: 106 wide, 8 around its content, its pointer 24 by
/// 14 with the tip at the top of the tab's label (measured on the
/// reference; `ToolWindowTabRenameActionBase`, a dialog balloon above the
/// label's top centre).
const BALLOON_WIDTH: f32 = 106.0;
const BALLOON_PADDING: f32 = 8.0;
const BALLOON_POINTER: (f32, f32) = (24.0, 14.0);
const BALLOON_RADIUS: f32 = 6.0;
const BALLOON_LABEL_LINE: f32 = 16.0;
/// The group of the tool window, for the buttons that show on hover.
const GROUP: &str = "terminal-tool-window";

/// What the tool window asks of its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminalToolWindowEvent {
    /// Start a new session in a new tab ("+", New Tab).
    NewTab,
    /// The user asked to close the tab with this id ([`TerminalTab::id`],
    /// the × of the tab, Close Tab, Close All): the owner may ask first
    /// when a command runs in its shell
    /// ([`SessionView::has_running_commands`]), then closes it with
    /// [`TerminalToolWindow::close_tab_by_id`].
    CloseRequested(u64),
    /// Hide the tool window ("−").
    Hide,
    /// The last tab was closed: the tool window hides.
    Emptied,
    /// A session has a new copy of its screen: draw the window now.
    Redraw,
    /// Remove from Sidebar: hide the tool window and its stripe button.
    RemoveFromSidebar,
    /// Settings in the options menu.
    Settings,
}

/// One tab: a session and its names.
pub struct TerminalTab {
    id: u64,
    /// The default name (`Local`, `Local (2)`).
    name: SharedString,
    /// The name the user gave the tab (Rename Session).
    user_title: Option<SharedString>,
    session: Entity<SessionView>,
    _subscription: Subscription,
}

impl TerminalTab {
    /// The tab's id: it stays while the tab is open, unlike its position.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The default name the tab got when it opened.
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    /// The title the running program set, while a command runs in the
    /// shell (the reference's default "Use application title as tab name:
    /// When command is running").
    fn program_title(&self, cx: &App) -> Option<String> {
        let session = self.session.read(cx);
        session
            .command_running()
            .then(|| session.title())
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
    }

    /// The tab's title: the user's name, else the program's title cut to
    /// [`TITLE_LIMIT`] characters, else the default name.
    pub fn title(&self, cx: &App) -> SharedString {
        if let Some(title) = &self.user_title {
            return title.clone();
        }
        match self.program_title(cx) {
            Some(title) => trim_middle(&title, TITLE_LIMIT).into(),
            None => self.name.clone(),
        }
    }

    /// The title in full (the tab's tooltip).
    pub fn full_title(&self, cx: &App) -> SharedString {
        if let Some(title) = &self.user_title {
            return title.clone();
        }
        self.program_title(cx)
            .map(SharedString::from)
            .unwrap_or_else(|| self.name.clone())
    }

    pub fn session(&self) -> &Entity<SessionView> {
        &self.session
    }
}

/// `text` cut to `max` characters with "…" in the middle, the second half
/// kept (`StringUtil.trimMiddle`).
pub fn trim_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let suffix = max / 2;
    let prefix = max - suffix - 1;
    let mut out: String = chars[..prefix].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - suffix..]);
    out
}

/// The Terminal tool window.
pub struct TerminalToolWindow {
    tabs: Vec<TerminalTab>,
    selected: Option<usize>,
    focus: FocusHandle,
    next_id: u64,
    /// "Close session when it ends" (`TerminalOptionsProvider.closeSessionOnLogout`).
    close_on_exit: bool,
    /// Where the selected tab's pill (or the grouped tabs' label) was
    /// drawn: the list of tabs opens below it, the rename balloon above.
    selected_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where the ⋮ button was drawn: its menu opens there from a command.
    options_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The open list of tabs.
    tab_list: Option<OpenPopup>,
    /// The open options menu (⋮).
    options_menu: Option<OpenPopup>,
    /// The open rename balloon.
    rename: Option<Rename>,
    /// Group Tabs: the header shows the selected tab's name and an arrow
    /// instead of a tab per session (`ToolWindowContentUiType.COMBO`).
    grouped: bool,
    /// The tool window has had several tabs: Group Tabs is offered from
    /// then on (`ToggleContentUiTypeAction.hadSeveralContents`).
    had_several: bool,
}

/// An open popup of the tool window.
struct OpenPopup {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

/// The rename balloon of a tab.
struct Rename {
    tab: u64,
    field: Entity<Editor>,
    /// An empty name was refused: the field is outlined in red until the
    /// text changes.
    error: bool,
    _subscription: Subscription,
}

impl EventEmitter<TerminalToolWindowEvent> for TerminalToolWindow {}

impl Focusable for TerminalToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A name for a new tab that no tab has: `base`, `base (2)`, `base (3)`, ...
/// (`UniqueNameGenerator.generateUniqueName` as `createDefaultTabName`
/// calls it with the tabs' titles).
pub fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let free = |name: &str| !taken.clone().into_iter().any(|t| t == name);
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|name| free(name))
        .expect("an unused name")
}

impl TerminalToolWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            selected: None,
            focus: cx.focus_handle(),
            next_id: 0,
            close_on_exit: true,
            selected_bounds: Rc::default(),
            options_bounds: Rc::default(),
            tab_list: None,
            options_menu: None,
            rename: None,
            grouped: false,
            had_several: false,
        }
    }

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The session of the selected tab.
    pub fn selected_session(&self) -> Option<&Entity<SessionView>> {
        self.selected
            .and_then(|i| self.tabs.get(i))
            .map(|t| &t.session)
    }

    /// Where activating the tool window moves the focus: the selected
    /// session, or the tool window itself while it has no tab.
    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        match self.selected_session() {
            Some(session) => session.focus_handle(cx),
            None => self.focus.clone(),
        }
    }

    pub fn set_close_on_exit(&mut self, close: bool) {
        self.close_on_exit = close;
    }

    /// A name for a new tab, unlike the titles of the open ones.
    pub fn new_tab_name(&self, cx: &App) -> String {
        let titles: Vec<SharedString> = self.tabs.iter().map(|t| t.title(cx)).collect();
        unique_name(DEFAULT_TAB_NAME, titles.iter().map(|t| t.as_ref()))
    }

    /// Adds a tab for `session` after the selected one and selects it; the
    /// focus moves into it when the tool window had it.
    pub fn add_tab(
        &mut self,
        session: Entity<SessionView>,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let name = name.unwrap_or_else(|| self.new_tab_name(cx)).into();
        let subscription =
            cx.subscribe_in(
                &session,
                window,
                move |this, _, event, window, cx| match event {
                    SessionViewEvent::Redraw => cx.emit(TerminalToolWindowEvent::Redraw),
                    SessionViewEvent::TitleChanged(_)
                    | SessionViewEvent::CommandRunningChanged(_) => cx.notify(),
                    SessionViewEvent::Exited(_) if this.close_on_exit => {
                        if let Some(index) = this.tabs.iter().position(|t| t.id == id) {
                            this.close(index, window, cx);
                        }
                    }
                    _ => {}
                },
            );
        let index = self.selected.map_or(self.tabs.len(), |s| s + 1);
        self.tabs.insert(
            index,
            TerminalTab {
                id,
                name,
                user_title: None,
                session: session.clone(),
                _subscription: subscription,
            },
        );
        self.had_several |= self.tabs.len() > 1;
        let had_focus = self.focus.contains_focused(window, cx);
        self.selected = Some(index);
        if had_focus {
            window.focus(&session.focus_handle(cx));
        }
        cx.notify();
        index
    }

    /// The position of the tab with this id.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The session of the tab with this id.
    pub fn session_of(&self, id: u64) -> Option<&Entity<SessionView>> {
        self.tabs.iter().find(|t| t.id == id).map(|t| &t.session)
    }

    /// The title of the tab with this id.
    pub fn title_of(&self, id: u64, cx: &App) -> Option<SharedString> {
        self.tabs.iter().find(|t| t.id == id).map(|t| t.title(cx))
    }

    /// Closes the tab with this id (see [`TerminalToolWindow::close`]); a
    /// tab that is gone already is left alone.
    pub fn close_tab_by_id(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.index_of(id) {
            self.close(index, window, cx);
        }
    }

    /// Closes a tab; its session ends with it. The focus, when it was in
    /// the tool window, goes to the tab selected next; the last tab closed
    /// hides the tool window.
    pub fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        let tab = self.tabs.remove(index);
        if self.rename.as_ref().is_some_and(|r| r.tab == tab.id) {
            self.rename = None;
        }
        self.selected = match self.selected {
            _ if self.tabs.is_empty() => None,
            Some(s) if s > index => Some(s - 1),
            Some(s) if s == index => Some(index.min(self.tabs.len() - 1)),
            other => other,
        };
        if had_focus {
            window.focus(&self.content_focus(cx));
        }
        if self.tabs.is_empty() {
            cx.emit(TerminalToolWindowEvent::Emptied);
        }
        cx.notify();
    }

    /// Selects a tab; the focus follows when it was in the tool window.
    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        self.selected = Some(index);
        if had_focus {
            window.focus(&self.tabs[index].session.focus_handle(cx));
        }
        cx.notify();
    }

    /// Selects the next tab (`delta` 1) or the previous one (-1), around
    /// the ends.
    pub fn select_relative(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        if let Some(s) = self.selected
            && n > 1
        {
            self.select((s as isize + delta).rem_euclid(n) as usize, window, cx);
        }
    }

    /// Renames a tab (the user's name, which stays while programs set
    /// their titles); an empty name is refused.
    pub fn rename(&mut self, index: usize, name: &str, cx: &mut Context<Self>) -> bool {
        let name = name.trim();
        match self.tabs.get_mut(index) {
            Some(tab) if !name.is_empty() => {
                tab.user_title = Some(name.to_owned().into());
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// The tool window has the focus and no popup of its own (a popup is a
    /// window of its own in the reference: the tool window is not active
    /// while one is open).
    fn is_active(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx) && !self.popup_open()
    }

    fn popup_open(&self) -> bool {
        self.tab_list.is_some() || self.options_menu.is_some() || self.rename.is_some()
    }

    /// The tabs and the selected session's state, for `terminal.state`.
    pub fn state_json(&self, window: &Window, cx: &App) -> Value {
        let mut state = match self.selected_session() {
            Some(session) => session.read(cx).state(),
            None => json!({}),
        };
        state["tabs"] = self
            .tabs
            .iter()
            .map(|t| {
                let s = t.session.read(cx);
                json!({
                    "name": t.title(cx).as_ref(),
                    "full_title": t.full_title(cx).as_ref(),
                    "default_name": t.name.as_ref(),
                    "title": s.title(),
                    "command_running": s.command_running(),
                    "working_directory": s.working_directory(),
                    "exited": s.exit_status().is_some(),
                })
            })
            .collect();
        state["selected"] = json!(self.selected);
        state["active"] = json!(self.is_active(window, cx));
        state["grouped"] = json!(self.grouped);
        // The open list of tabs and the open options menu, as a menu's
        // state (its rows and the selected one).
        state["tab_list"] = self
            .tab_list
            .as_ref()
            .map_or(Value::Null, |open| open.menu.read(cx).state_json());
        state["options_menu"] = self
            .options_menu
            .as_ref()
            .map_or(Value::Null, |open| open.menu.read(cx).state_json());
        state["rename"] = self.rename.as_ref().map_or(Value::Null, |r| {
            json!({
                "tab": self.index_of(r.tab),
                "text": r.field.read(cx).text(),
                "error": r.error,
            })
        });
        state
    }

    // ---- commands -------------------------------------------------------

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TerminalToolWindowEvent::NewTab);
    }

    fn close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.selected.and_then(|s| self.tabs.get(s)) {
            cx.emit(TerminalToolWindowEvent::CloseRequested(tab.id));
        }
    }

    /// Close All (`TabbedContentAction.CloseAllAction`): every tab is asked
    /// to close.
    fn close_all_tabs(&mut self, _: &CloseAllTabs, _: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            cx.emit(TerminalToolWindowEvent::CloseRequested(id));
        }
    }

    fn select_tab(&mut self, action: &SelectTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = action.index {
            self.select(index, window, cx);
        }
    }

    /// Show List of Tabs (`ShowContentAction`, enabled with more than one
    /// tab): the tabs' titles in a list popup with the speed search, the
    /// selected tab preselected, under the selected tab; choosing a row
    /// selects that tab (`SelectContentStep`).
    fn show_tab_list(&mut self, _: &ShowTabList, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let items = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| MenuItem::action(tab.title(cx), SelectTab { index: Some(i) }))
            .collect();
        let anchor = self.selected_bounds.get();
        // Grouped, the list is as wide as the label it opens from
        // (`ComboContentLayout.showContentPopup`).
        let min_width = if self.grouped {
            f32::from(anchor.size.width).max(TAB_LIST_MIN_WIDTH)
        } else {
            TAB_LIST_MIN_WIDTH
        };
        let options = PopupOptions {
            speed_search: true,
            selected: self.selected,
            min_width: Some(min_width),
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let header_bottom = px((HEADER_HEIGHT - HEADER_TAB_HEIGHT) / 2.0) + px(1.0);
        let position = if anchor.size.width > px(0.0) {
            let left = if self.grouped {
                anchor.left()
            } else {
                anchor.left() + px(TAB_LIST_LEFT)
            };
            point(left, anchor.bottom() + header_bottom)
        } else {
            point(px(0.0), px(HEADER_HEIGHT + 1.0))
        };
        self.tab_list = Some(self.open_popup(menu, position, cx));
        cx.notify();
    }

    /// Keeps an open popup until it is dismissed.
    fn open_popup(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> OpenPopup {
        let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this.tab_list.as_ref().is_some_and(|o| o.menu == menu) {
                this.tab_list = None;
            }
            if this.options_menu.as_ref().is_some_and(|o| o.menu == menu) {
                this.options_menu = None;
            }
            cx.notify();
        });
        OpenPopup {
            menu,
            position,
            _subscription: subscription,
        }
    }

    /// The header's ⋮ menu (`ToolWindowImpl.GearActionGroup` with the
    /// terminal's `Terminal.ToolWindowActions`), at `position`.
    fn open_options_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let unavailable = |labels: &[&'static str]| -> Vec<MenuItem> {
            labels.iter().map(|l| MenuItem::unavailable(*l)).collect()
        };
        let items = vec![
            MenuItem::submenu(
                "Terminal Engine",
                vec![
                    // The reference says "Reworked 2025"; the year is left
                    // out here.
                    MenuItem::action(
                        "Reworked",
                        SelectEngine {
                            engine: Some("reworked".into()),
                        },
                    )
                    .icon(IconName::CHECKED),
                    MenuItem::unavailable("Classic"),
                ],
            ),
            MenuItem::action("Settings", ShowSettings).icon(IconName::SETTINGS_SMALL),
            MenuItem::separator(),
            MenuItem::action("Close All", CloseAllTabs),
            {
                let item = MenuItem::action("Group Tabs", GroupTabs).hidden(!self.had_several);
                if self.grouped {
                    item.icon(IconName::CHECKED)
                } else {
                    item
                }
            },
            // The view modes, moving and resizing of tool windows are not
            // there yet; the tool window is docked and pinned.
            MenuItem::submenu(
                "View Mode",
                std::iter::once(MenuItem::unavailable("Dock Pinned").icon(IconName::CHECKED))
                    .chain(unavailable(&["Dock Unpinned", "Undock", "Float", "Window"]))
                    .collect(),
            ),
            MenuItem::submenu(
                "Move to",
                unavailable(&[
                    "Left Top",
                    "Left Bottom",
                    "Bottom Left",
                    "Bottom Right",
                    "Right Bottom",
                    "Right Top",
                ]),
            ),
            MenuItem::submenu(
                "Resize",
                unavailable(&[
                    "Stretch to Left",
                    "Stretch to Right",
                    "Stretch to Top",
                    "Stretch to Bottom",
                    "Maximize Tool Window",
                ]),
            ),
            MenuItem::separator(),
            MenuItem::action("Remove from Sidebar", RemoveFromSidebar),
        ];
        // A click in the header activates the tool window: the chosen
        // command runs in it.
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.content_focus(cx));
        }
        let menu = ContextMenu::build(items, window, cx);
        self.options_menu = Some(self.open_popup(menu, position, cx));
        cx.notify();
    }

    fn show_options(&mut self, _: &ShowOptions, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.options_bounds.get();
        let position = if anchor.size.width > px(0.0) {
            anchor.center()
        } else {
            point(px(0.0), px(HEADER_HEIGHT))
        };
        self.open_options_menu(position, window, cx);
    }

    fn group_tabs(&mut self, _: &GroupTabs, _: &mut Window, cx: &mut Context<Self>) {
        self.grouped = !self.grouped;
        cx.notify();
    }

    fn remove_from_sidebar(
        &mut self,
        _: &RemoveFromSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(TerminalToolWindowEvent::RemoveFromSidebar);
    }

    /// Only the reworked engine exists, and it is in use: nothing changes.
    fn select_engine(&mut self, _: &SelectEngine, _: &mut Window, _: &mut Context<Self>) {}

    fn show_settings(&mut self, _: &ShowSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TerminalToolWindowEvent::Settings);
    }

    fn rename_tab(&mut self, action: &RenameTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.selected else {
            return;
        };
        match &action.name {
            Some(name) => {
                self.rename(s, name, cx);
            }
            None => self.open_rename(s, window, cx),
        }
    }

    /// The rename balloon above the tab (`ToolWindowTabRenameActionBase`):
    /// "Session name:" over a field with the tab's title, selected.
    fn open_rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let (id, title) = (tab.id, tab.title(cx));
        let field = editor::field::new(&title, None, true, window, cx);
        field.update(cx, |e, cx| e.set_text_selected(&title, cx));
        let subscription = cx.subscribe(&field, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_))
                && let Some(rename) = &mut this.rename
                && rename.error
            {
                rename.error = false;
                cx.notify();
            }
        });
        window.focus(&field.focus_handle(cx));
        self.rename = Some(Rename {
            tab: id,
            field,
            error: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Enter in the balloon: an empty name is refused with an error
    /// outline; otherwise the tab is renamed and the balloon closes.
    fn rename_confirm(&mut self, _: &RenameConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &mut self.rename else {
            return;
        };
        let text = rename.field.read(cx).text();
        if text.trim().is_empty() {
            rename.error = true;
            cx.notify();
            return;
        }
        let tab = rename.tab;
        if let Some(index) = self.index_of(tab) {
            self.rename(index, &text, cx);
        }
        self.close_rename(window, cx);
    }

    fn rename_cancel(&mut self, _: &RenameCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close_rename(window, cx);
    }

    /// The balloon closes; the focus goes to the selected session.
    fn close_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            window.focus(&self.content_focus(cx));
            cx.notify();
        }
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_relative(1, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_relative(-1, window, cx);
    }

    // ---- drawing --------------------------------------------------------

    /// A canvas over an element that records where it was drawn.
    fn bounds_probe(cell: &Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
        let cell = cell.clone();
        canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    fn render_tab(&self, index: usize, active: bool, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let selected = self.selected == Some(index);
        let hover = theme.color(UiRole::TabHover);
        let text = if selected {
            theme.color(UiRole::TabActiveText)
        } else {
            theme.color(UiRole::TabInactiveText)
        };
        let close = div()
            .id(("terminal-tab-close", index))
            .debug_selector(move || format!("terminal-tab-close-{index}"))
            .flex_none()
            .size(px(16.0))
            .rounded(px(3.0))
            .hover(move |s| s.bg(hover))
            .child(Icon::new(IconName::CLOSE_SMALL))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if let Some(tab) = this.tabs.get(index) {
                    cx.emit(TerminalToolWindowEvent::CloseRequested(tab.id));
                }
            }));
        header_tab(("terminal-tab", index), selected, active, cx)
            .debug_selector(move || format!("terminal-tab-{index}"))
            .ml(px(TAB_OFFSET))
            .mr(px(TAB_OFFSET_RIGHT))
            .pl(px(TAB_TEXT - 1.0))
            .pr(px(TAB_CLOSE_RIGHT))
            .gap(px(TAB_CLOSE_GAP))
            .child(
                div()
                    .whitespace_nowrap()
                    .text_color(text)
                    .child(tab.title(cx)),
            )
            .child(close)
            .when(selected, |d| {
                d.relative()
                    .child(Self::bounds_probe(&self.selected_bounds))
            })
            // No tooltip while a popup of the tool window is open.
            .when(!self.popup_open(), |d| {
                d.tooltip(Tooltip::text(tab.full_title(cx), None))
            })
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                this.select(index, window, cx);
                if let Some(session) = this.selected_session() {
                    window.focus(&session.focus_handle(cx));
                }
                // A double click renames the tab.
                if e.click_count() == 2 {
                    this.open_rename(index, window, cx);
                }
            }))
            .into_any_element()
    }

    /// Group Tabs: the selected tab's title with an arrow; a click opens
    /// the list of tabs (`ContentComboLabel`).
    fn render_grouped(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .selected
            .and_then(|s| self.tabs.get(s))
            .map(|t| t.title(cx))
            .unwrap_or_default();
        header_tab("terminal-tabs-group", false, false, cx)
            .debug_selector(|| "terminal-tabs-group".into())
            .relative()
            .ml(px(TAB_OFFSET))
            .mr(px(TAB_OFFSET_RIGHT))
            .pl(px(TAB_TEXT - 1.0))
            .pr(px(TAB_CLOSE_RIGHT))
            .gap(px(4.0))
            .child(div().whitespace_nowrap().child(title))
            .child(Icon::new(IconName::CHEVRON_DOWN))
            .child(Self::bounds_probe(&self.selected_bounds))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                if this.tab_list.is_none() {
                    this.show_tab_list(&ShowTabList, window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.is_active(window, cx);
        let tabs: Vec<AnyElement> = if self.grouped && self.tabs.len() > 1 {
            vec![self.render_grouped(cx)]
        } else {
            (0..self.tabs.len())
                .map(|i| self.render_tab(i, active, cx))
                .collect()
        };
        // The buttons show while the tool window is active, has a popup
        // open or is under the pointer.
        let show_buttons = active || self.popup_open();
        let buttons = |d: gpui::Div| {
            d.flex()
                .flex_row()
                .flex_none()
                .items_center()
                .when(!show_buttons, |d| {
                    d.invisible().group_hover(GROUP, |s| s.visible())
                })
        };
        let theme = cx.theme();
        let options_bounds = self.options_bounds.clone();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(HEADER_HEIGHT + 1.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
            .pl(px(TITLE_LEFT))
            .pr(px(8.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .mr(px(TITLE_RIGHT))
                    .child("Terminal"),
            )
            .child(
                div()
                    .id("terminal-tabs")
                    .debug_selector(|| "terminal-tabs".into())
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w(px(0.0))
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                buttons(div())
                    .gap(px(2.0))
                    .child(
                        icon_button(
                            "terminal-new-tab",
                            IconName::ADD,
                            px(BUTTON),
                            px(ICON),
                            false,
                            cx,
                        )
                        .debug_selector(|| "terminal-new-tab".into())
                        .tooltip(Tooltip::text("New Tab", None))
                        .on_click(cx.listener(
                            |_, _: &ClickEvent, _, cx| cx.emit(TerminalToolWindowEvent::NewTab),
                        )),
                    )
                    .child(
                        icon_button(
                            "terminal-new-session",
                            IconName::CHEVRON_DOWN,
                            px(BUTTON),
                            px(ICON),
                            false,
                            cx,
                        )
                        .tooltip(Tooltip::text("New Predefined Session", None)),
                    ),
            )
            .child(div().flex_1())
            .child(
                buttons(div())
                    .gap(px(2.0))
                    .child(
                        icon_button(
                            "terminal-options",
                            IconName::MORE_VERTICAL,
                            px(BUTTON),
                            px(ICON),
                            false,
                            cx,
                        )
                        .debug_selector(|| "terminal-options".into())
                        .relative()
                        .tooltip(Tooltip::text("Options", None))
                        .child(Self::bounds_probe(&options_bounds))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                if this.options_menu.is_none() {
                                    this.open_options_menu(e.position, window, cx);
                                }
                            }),
                        ),
                    )
                    .child(
                        icon_button(
                            "terminal-hide",
                            IconName::HIDE,
                            px(BUTTON),
                            px(ICON),
                            false,
                            cx,
                        )
                        .debug_selector(|| "terminal-hide".into())
                        .tooltip(Tooltip::text("Hide", None))
                        .on_click(cx.listener(
                            |_, _: &ClickEvent, _, cx| cx.emit(TerminalToolWindowEvent::Hide),
                        )),
                    ),
            )
    }

    /// The rename balloon over the selected tab: its pointer's tip at the
    /// top of the tab's label, the balloon centred over it.
    fn render_rename(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rename = self.rename.as_ref()?;
        let pill = self.selected_bounds.get();
        if pill.size.width <= px(0.0) {
            return None;
        }
        let theme = cx.theme();
        let fill = theme.color(UiRole::SurfaceRaised);
        let border = theme.color(UiRole::PopupBorder);
        let (pointer_w, pointer_h) = (px(BALLOON_POINTER.0), px(BALLOON_POINTER.1));
        let tip = point(
            pill.center().x,
            pill.top() - px((HEADER_HEIGHT - HEADER_TAB_HEIGHT) / 2.0),
        );
        let field = editor::field::frame(&rename.field, true, rename.error, window, cx)
            .bg(theme.color(UiRole::Input));
        let content = div()
            .id("terminal-rename")
            .debug_selector(|| "terminal-rename".into())
            .key_context(RENAME_CONTEXT)
            .on_action(cx.listener(Self::rename_confirm))
            .on_action(cx.listener(Self::rename_cancel))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_rename(window, cx)))
            .w(px(BALLOON_WIDTH))
            .p(px(BALLOON_PADDING))
            .pb(px(BALLOON_PADDING + 1.0))
            .flex()
            .flex_col()
            // The label, 2 above the field (`SwingHelper.newLeftAlignedVerticalPanel`).
            .gap(px(2.0))
            .rounded(px(BALLOON_RADIUS))
            .border_1()
            .border_color(border)
            .bg(fill)
            .shadow(ui::components::popup_shadow())
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .line_height(px(BALLOON_LABEL_LINE))
                    .whitespace_nowrap()
                    .child("Session name:"),
            )
            .child(field);
        // The pointer, over the balloon's border where it joins it.
        let pointer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let (x0, x1) = (bounds.left(), bounds.right());
                let mid = bounds.left() + pointer_w / 2.0;
                let (base, tip) = (bounds.top() - px(1.0), bounds.bottom());
                let mut path = PathBuilder::fill();
                path.move_to(point(x0, base));
                path.line_to(point(mid, tip));
                path.line_to(point(x1, base));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, fill);
                }
                let mut edge = PathBuilder::stroke(px(1.0));
                edge.move_to(point(x0, base));
                edge.line_to(point(mid, tip));
                edge.line_to(point(x1, base));
                if let Ok(path) = edge.build() {
                    window.paint_path(path, border);
                }
            },
        )
        .ml(px(BALLOON_WIDTH / 2.0) - pointer_w / 2.0)
        .w(pointer_w)
        .h(pointer_h);
        Some(
            deferred(
                anchored()
                    .position(point(tip.x - px(BALLOON_WIDTH / 2.0), tip.y))
                    .anchor(Corner::BottomLeft)
                    .snap_to_window_with_margin(px(4.0))
                    .child(div().flex().flex_col().child(content).child(pointer)),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

impl Render for TerminalToolWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx).into_any_element();
        let body = self.selected_session().cloned();
        let popups: Vec<AnyElement> = [&self.tab_list, &self.options_menu]
            .into_iter()
            .flatten()
            .map(|open| ui::menu::overlay(&open.menu, open.position, window))
            .collect();
        let rename = self.render_rename(window, cx);
        div()
            .id("terminal-tool-window-content")
            .group(GROUP)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::rename_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::select_tab))
            .on_action(cx.listener(Self::show_tab_list))
            .on_action(cx.listener(Self::show_options))
            .on_action(cx.listener(Self::group_tabs))
            .on_action(cx.listener(Self::remove_from_sidebar))
            .on_action(cx.listener(Self::select_engine))
            .on_action(cx.listener(Self::show_settings))
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(
                div()
                    .id("terminal-session")
                    .debug_selector(|| "terminal-session".into())
                    .flex_1()
                    .min_h(px(0.0))
                    .children(body),
            )
            .children(popups)
            .children(rename)
    }
}

#[cfg(test)]
mod tests {
    use super::{trim_middle, unique_name};

    #[test]
    fn unique_names_take_the_first_free_number() {
        assert_eq!(unique_name("Local", []), "Local");
        assert_eq!(unique_name("Local", ["Local"]), "Local (2)");
        assert_eq!(
            unique_name("Local", ["Local", "Local (2)", "build"]),
            "Local (3)"
        );
        assert_eq!(unique_name("Local", ["Local (2)"]), "Local");
    }

    #[test]
    fn long_titles_are_cut_in_the_middle() {
        assert_eq!(trim_middle("vim", 30), "vim");
        let thirty = "a".repeat(30);
        assert_eq!(trim_middle(&thirty, 30), thirty);
        // 14 characters, the ellipsis, the last 15 (`StringUtil.trimMiddle`).
        assert_eq!(
            trim_middle("python -m http.server 8000 --bind 127.0.0.1", 30),
            "python -m http…-bind 127.0.0.1"
        );
    }
}
