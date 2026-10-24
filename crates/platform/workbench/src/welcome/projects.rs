//! The Welcome screen's left panel, "Projects and Files" (the reference's
//! `WelcomeScreenLeftPanel` of `intellij.platform.ide.nonModalWelcomeScreen`,
//! IntelliJ Community, Apache-2.0): what the project tool window shows
//! while no project is open. A column of buttons (Open…, New…, Clone
//! Repository…; `createToolWindowWelcomeScreenVerticalToolbar`), a line,
//! the "Search projects" field, a line, and the recent projects
//! (`RecentProjectFilteringTree`; without any, "No recent projects" a
//! third of the way down, where `StatusText` puts it).
//!
//! The list starts 10 below the line (6 below the line's row, 4 of the
//! tree's border); its rows are 12 in from the sides (`Tree.border`).
//!
//! A recent project's row (`RecentProjectComponent`): a border of 4, the
//! avatar 6 in from it (top and left) and 8 before the name panel, which
//! has 4 around it, the name and below it, 4 apart, the folder and the
//! branch of its Git work tree (after the branch icon, 4 before the text)
//! in the comment colour. A click opens the project; one on a folder that
//! is gone asks "Reopen Project" whether to take it off the list. The
//! selected row (light selection, `Table.lightSelectionBackground`) shows
//! a button of 22 centred on its right end, 12 in from the selection's
//! edge (the renderer spans to the tree's edge, 12 beyond the selection,
//! less its border of 4 and the button's gap of 20): the "more" icon with
//! the row's menu (`WelcomeScreenRecentProjectActionGroup`: Open Selected,
//! Reveal in Finder, Copy Path, New Project Group, Move to Group, Set
//! Custom Project Icon…, Remove from Recent Projects…; groups and custom
//! icons are not here yet, their rows are greyed), or for a folder that is
//! gone a cross (`Welcome.RecentProjects.Remove`) that takes it off the
//! list after asking; the button keeps its hover background while its
//! menu is open. A gone folder's row is greyed, its avatar desaturated. A
//! right click opens the menu too. The pointer over a row is a hand. The
//! search field filters the rows by name and folder.
//!
//! The keyboard (`RecentProjectFilteringTree`, `FilteringTree`): one row is
//! selected, as the reference's tree has it: the row under the pointer (the
//! tree selects the row the pointer moves over and clears the selection
//! when it leaves), the first row at the start, else the one the keys
//! moved to; the selected row shows its button. Up and Down move the
//! selection, from the list and from the search field alike, without
//! wrapping (Up with no selection goes to the last row, Down to the
//! first); Home and End go to the first and the last row. Enter opens the
//! selected project. Delete, Backspace and Command+Backspace (`$Delete`)
//! in the list, Option+Delete in the search field, take it off the list
//! after asking; the shortcut of Copy Paths copies its folder. Escape in
//! the search field clears its text, and moves the focus to the list when
//! the text is empty. When the search text changes, the selected row stays
//! if it is still listed, else the first one is selected.
//!
//! Metrics of the reference: the buttons 34 high, 8 apart, 20 from the
//! sides and 8 below the top, corners of 8, the icon 14 from the left and
//! 12 before the label (`DisclosureButton`, `DarculaDisclosureButtonUI`);
//! their background white at 7 % (dark themes) or black at 4 %, 5 % more
//! under the mouse. The line 26 below the last button, the search row 42
//! high (measured in the reference).

use std::path::PathBuf;

use gpui::{
    AnyElement, App, ClickEvent, Context, DismissEvent, Entity, FocusHandle, Focusable as _, Hsla,
    IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

use crate::commands::OpenFolder;
use crate::recent_projects::{self, RecentProject};
use crate::workbench::project_widget::{
    CopyRecentPath, OpenRecent, REVEAL, RemoveRecent, RevealRecent,
};

ui::ui_commands! {
    COMMANDS, scope = "Welcome";
    /// The row above.
    Up => "welcome-projects.up", "Up",
        "Projects and Files: selects the recent project above (the last one when none is selected).";
    /// The row below.
    Down => "welcome-projects.down", "Down",
        "Projects and Files: selects the recent project below (the first one when none is selected).";
    /// The first row.
    First => "welcome-projects.first", "First",
        "Projects and Files: selects the first recent project.";
    /// The last row.
    Last => "welcome-projects.last", "Last",
        "Projects and Files: selects the last recent project.";
    /// Open the selected row.
    OpenSelected => "welcome-projects.open", "Open Selected",
        "Projects and Files: opens the selected recent project (`WelcomeScreen.OpenSelected`).";
    /// Copy the selected row's folder.
    CopySelectedPath => "welcome-projects.copy-path", "Copy Path",
        "Projects and Files: copies the folder of the selected recent project (`WelcomeScreen.CopyProjectPath`).";
    /// Remove the selected row.
    RemoveSelected => "welcome-projects.remove", "Remove from Recent Projects\u{2026}",
        "Projects and Files: asks, then takes the selected recent project off the list (`WelcomeScreen.RemoveSelected`).";
    /// Escape in the search field.
    Escape => "welcome-projects.escape", "Clear the Search",
        "Projects and Files, in the search field: clears the text, or moves the focus to the list when the text is empty.";
}

/// `welcome.screen.project.view.title` (`NonModalWelcomeScreenBundle`).
pub const TITLE: &str = "Projects and Files";
/// `action.Tabbed.WelcomeScreen.NonModal.OpenProject.text`.
pub const OPEN: &str = "Open\u{2026}";
/// `action.NonModalWelcomeScreen.LeftTabActions.New.Action.text`.
pub const NEW: &str = "New\u{2026}";
/// `action.ProjectFromVersionControl.ProjectWidgetPopup.text`.
pub const CLONE: &str = "Clone Repository\u{2026}";
/// The search field's placeholder.
pub const SEARCH: &str = "Search projects";
/// `welcome.screen.no.recent.projects`.
pub const NO_RECENT: &str = "No recent projects";
/// The tooltip of what has nothing behind it yet.
pub const NOT_AVAILABLE: &str = "Not available yet";

const BUTTON_HEIGHT: f32 = 34.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_RADIUS: f32 = 8.0;
const BUTTONS_X: f32 = 20.0;
const BUTTONS_TOP: f32 = 8.0;
const BUTTON_ICON_X: f32 = 14.0;
const BUTTON_ICON_GAP: f32 = 12.0;
const LINE_TOP: f32 = 26.0;
const SEARCH_HEIGHT: f32 = 42.0;
const SEARCH_ICON_X: f32 = 26.0;
const SEARCH_ICON_GAP: f32 = 8.0;
/// The list's top and bottom (see the module's notes).
const LIST_TOP: f32 = 10.0;
const LIST_BOTTOM: f32 = 4.0;
/// A project's row (see the module's notes) and the selection's inset
/// from the panel's sides (the tree's, as in the project tree).
const ROW_BORDER: f32 = 4.0;
const ROW_ICON: f32 = 6.0;
const ROW_ICON_GAP: f32 = 8.0;
const ROW_TEXT: f32 = 4.0;
const ROW_LINE_GAP: f32 = 4.0;
const ROW_NAME_LINE: f32 = 17.0;
const ROW_PATH_LINE: f32 = 15.0;
/// The branch's line: a label with a 16 icon, 4 before the text.
const ROW_BRANCH_LINE: f32 = 16.0;
const BRANCH_ICON: f32 = 16.0;
const BRANCH_ICON_GAP: f32 = 4.0;
const AVATAR: f32 = 20.0;
const ROW_BUTTON: f32 = 22.0;
const ROW_INSET: f32 = 12.0;
/// The button's right end from the selection's (see the module's notes).
const ROW_BUTTON_RIGHT: f32 = 20.0 + ROW_BORDER - ROW_INSET;
/// The button's corners (`ActionsButton.selectionArc` 6 is a diameter).
const ROW_BUTTON_RADIUS: f32 = 3.0;
/// The cross's size (a 10 by 10 icon).
const REMOVE_ICON: f32 = 10.0;
/// `recent.project.unavailable`, after a gone folder in its tooltip.
pub const UNAVAILABLE: &str = "(unavailable)";
/// `action.presentation.OpenSelectedProjectsAction.text.open.selected`.
pub const OPEN_SELECTED: &str = "Open Selected";
/// `action.WelcomeScreen.CopyProjectPath.text`.
pub const COPY_PATH: &str = "Copy Path";
/// `action.WelcomeScreen.RemoveSelected.text`.
pub const REMOVE: &str = "Remove from Recent Projects\u{2026}";
/// `action.WelcomeScreen.NewGroup.text`.
pub const NEW_GROUP: &str = "New Project Group";
/// `group.WelcomeScreen.MoveToGroup.text`.
pub const MOVE_TO_GROUP: &str = "Move to Group";
/// `action.WelcomeScreen.ChangeProjectIcon.text`.
pub const CHANGE_ICON: &str = "Set Custom Project Icon\u{2026}";

/// A button of the column: its label, icon and what it runs (none yet:
/// drawn disabled, with [`NOT_AVAILABLE`] as its tooltip).
#[derive(Clone, Copy)]
struct Button {
    label: &'static str,
    icon: IconName,
    enabled: bool,
}

/// Open… (`WelcomeScreen.OpenDirectoryProject`: the folder icon on the
/// non-modal Welcome screen), New… (`General.Add`) and Clone Repository…
/// (`General.Vcs`). New… lists the New Project wizard and the new file
/// kinds in the reference, Clone Repository… opens its clone dialog: we
/// have neither yet.
const BUTTONS: [Button; 3] = [
    Button {
        label: OPEN,
        icon: IconName::FOLDER,
        enabled: true,
    },
    Button {
        label: NEW,
        icon: IconName::ADD,
        enabled: false,
    },
    Button {
        label: CLONE,
        icon: IconName::VCS,
        enabled: false,
    },
];

/// Opening a recent project from the list (a folder that is gone asks
/// the Welcome tab's question).
fn open_action(path: String) -> OpenRecent {
    OpenRecent {
        path,
        welcome: Some(true),
    }
}

pub struct WelcomeProjects {
    search: Entity<editor::Editor>,
    /// The list's focus (the project tool window's: the panel hands it
    /// over while it shows this list).
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The selected row: the one under the pointer, or the one the keys
    /// moved to (a folder that is no longer listed selects nothing).
    selected: Option<PathBuf>,
    /// The first row was selected (`selectLastOpenedProjectOrTheFirstInTree`).
    initially_selected: bool,
    /// The search field's text when the list was last filtered.
    searched: String,
    /// A row's menu and where it opened.
    menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    /// The menu was opened from the row's button (which keeps its hover
    /// background meanwhile).
    menu_from_button: bool,
    /// The row pressed last has no tooltip until the pointer leaves it (a
    /// press hides the tooltip, `IdeTooltipManager`; one waiting to show
    /// would otherwise come up over the dialog or the project).
    pressed: Option<PathBuf>,
}

impl WelcomeProjects {
    /// The list takes `focus` as its own (the panel's, what the tool window
    /// focuses).
    pub fn new(focus: FocusHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = editor::field::new("", Some(SEARCH.into()), true, window, cx);
        cx.observe(&search, |this, _, cx| this.search_changed(cx))
            .detach();
        cx.observe_global::<recent_projects::Store>(|this, cx| {
            this.list_changed(cx);
            recent_projects::check_folders(cx);
        })
        .detach();
        cx.observe_global::<recent_projects::FolderState>(|_, cx| cx.notify())
            .detach();
        // The folders are looked at again when the window comes to the
        // front (the branches' cache refreshes on activation).
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                recent_projects::check_folders(cx);
            }
        })
        .detach();
        let mut this = Self {
            search,
            focus,
            scroll: ScrollHandle::new(),
            selected: None,
            initially_selected: false,
            searched: String::new(),
            menu: None,
            menu_from_button: false,
            pressed: None,
        };
        this.list_changed(cx);
        recent_projects::check_folders(cx);
        this
    }

    /// The search field (what the keys of the field go to).
    pub fn search(&self) -> &Entity<editor::Editor> {
        &self.search
    }

    /// The recent projects changed: the selection of a folder that is gone
    /// from the list is dropped (the tree's selection model does that for a
    /// removed node), and the first row is selected once there is one.
    fn list_changed(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths(cx);
        if self.selected_ix(&paths).is_none() {
            self.selected = None;
        }
        if !self.initially_selected && !paths.is_empty() {
            self.initially_selected = true;
            self.selected = paths.first().cloned();
        }
        cx.notify();
    }

    /// The search text changed (`FilteringSpeedSearch.updateSelection`): the
    /// selected row stays when it is still listed, else the first row is
    /// selected.
    fn search_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).text();
        if text != self.searched {
            self.searched = text;
            if !self.searched.trim().is_empty() {
                let paths = self.paths(cx);
                if self.selected_ix(&paths).is_none() {
                    self.select(&paths, 0);
                }
            }
        }
        cx.notify();
    }

    /// The folders of the rows shown, in order.
    fn paths(&self, cx: &App) -> Vec<PathBuf> {
        self.shown(cx).into_iter().map(|p| p.path).collect()
    }

    /// The selected row's place in `paths`.
    fn selected_ix(&self, paths: &[PathBuf]) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        paths.iter().position(|p| p == selected)
    }

    /// Selects row `ix` of `paths` and scrolls it into view.
    fn select(&mut self, paths: &[PathBuf], ix: usize) {
        if let Some(path) = paths.get(ix) {
            self.selected = Some(path.clone());
            self.scroll.scroll_to_item(ix);
        }
    }

    /// The selected row's folder, as a string for the commands.
    fn selected_folder(&self, cx: &App) -> Option<String> {
        let paths = self.paths(cx);
        let ix = self.selected_ix(&paths)?;
        Some(paths[ix].to_string_lossy().into_owned())
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.paths(cx);
        let ix = match self.selected_ix(&paths) {
            Some(ix) => ix.saturating_sub(1),
            None => paths.len().saturating_sub(1),
        };
        self.select(&paths, ix);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.paths(cx);
        let ix = match self.selected_ix(&paths) {
            Some(ix) => (ix + 1).min(paths.len() - 1),
            None => 0,
        };
        self.select(&paths, ix);
        cx.notify();
    }

    fn first(&mut self, _: &First, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.paths(cx);
        self.select(&paths, 0);
        cx.notify();
    }

    fn last(&mut self, _: &Last, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.paths(cx);
        self.select(&paths, paths.len().saturating_sub(1));
        cx.notify();
    }

    /// Enter (`activateItems`): what a click on the row does.
    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_folder(cx) {
            window.dispatch_action(Box::new(open_action(path)), cx);
        }
    }

    /// Copy Paths: the selected row's folder (the tree's data provider has
    /// no item in the search field, where the text is copied instead).
    fn copy_path(&mut self, _: &CopySelectedPath, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
        if let Some(path) = self.selected_folder(cx) {
            window.dispatch_action(Box::new(CopyRecentPath { path }), cx);
        }
    }

    /// `removeItem`: asks first.
    fn remove_selected(&mut self, _: &RemoveSelected, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_folder(cx) {
            window.dispatch_action(Box::new(RemoveRecent { path }), cx);
        }
    }

    /// Escape in the search field: `ClearText` while there is text, else
    /// the focus goes to the tree (`FilteringTree.installSearchField`).
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).text().is_empty() {
            window.focus(&self.focus);
        } else {
            self.search
                .update(cx, |search, cx| search.set_text_selected("", cx));
        }
    }

    /// The recent projects the search field lets through, by name or
    /// folder (`*` and the text, case ignored, as the speed search does).
    fn shown(&self, cx: &App) -> Vec<RecentProject> {
        let text = self.search.read(cx).text();
        let text = text.trim();
        let matcher =
            (!text.is_empty()).then(|| Matcher::new(&format!("*{text}"), MatchingMode::IgnoreCase));
        recent_projects::list(cx)
            .projects()
            .iter()
            .filter(|p| {
                matcher.as_ref().is_none_or(|m| {
                    m.matches(&p.name) || m.matches(&project::display_path(&p.path))
                })
            })
            .cloned()
            .collect()
    }

    /// What the panel shows, for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let folders = recent_projects::folders(cx);
        json!({
            "title": TITLE,
            "buttons": BUTTONS.iter().map(|b| json!({
                "label": b.label,
                "enabled": b.enabled,
            })).collect::<Vec<_>>(),
            "search": self.search.read(cx).text(),
            "projects": self.shown(cx).iter().map(|p| json!({
                "name": p.name,
                "path": project::display_path(&p.path),
                "avatar": ui::avatar::initials(&p.name),
                "gone": folders.gone.contains(&p.path),
                "branch": folders.branches.get(&p.path),
            })).collect::<Vec<_>>(),
            "selected": self
                .selected_folder(cx)
                .map(|p| project::display_path(std::path::Path::new(&p))),
            "menu": self.menu.as_ref().map(|(m, _, _)| m.read(cx).state_json()),
            "empty_text": NO_RECENT,
        })
    }

    /// The row's menu at `position` (`WelcomeScreenRecentProjectActionGroup`:
    /// the custom actions of other providers are none; groups and custom
    /// icons are not here yet). The shortcuts are the list's (Copy Paths,
    /// `$Delete`).
    fn open_menu(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        from_button: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = path.to_string_lossy().into_owned();
        let shortcut = |item: MenuItem, action: &dyn gpui::Action| match ui::keys::shortcut_in(
            action,
            &self.focus,
            window,
        ) {
            Some(text) => item.shortcut(text),
            None => item,
        };
        let items = vec![
            MenuItem::action(OPEN_SELECTED, open_action(path.clone())),
            MenuItem::action(REVEAL, RevealRecent { path: path.clone() }),
            shortcut(
                MenuItem::action(COPY_PATH, CopyRecentPath { path: path.clone() }),
                &CopySelectedPath,
            ),
            MenuItem::separator(),
            MenuItem::unavailable(NEW_GROUP),
            MenuItem::submenu(MOVE_TO_GROUP, Vec::new()).enabled(false),
            MenuItem::separator(),
            MenuItem::unavailable(CHANGE_ICON),
            MenuItem::separator(),
            shortcut(
                MenuItem::action(REMOVE, RemoveRecent { path }),
                &RemoveSelected,
            ),
        ];
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            this.menu_from_button = false;
            cx.notify();
        });
        self.menu = Some((menu, position, subscription));
        self.menu_from_button = from_button;
        cx.notify();
    }

    /// A recent project's row.
    fn render_row(
        &self,
        ix: usize,
        p: &RecentProject,
        folders: &recent_projects::Folders,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let gone = folders.gone.contains(&p.path);
        let branch = folders.branches.get(&p.path).cloned();
        let selected = self.selected.as_ref() == Some(&p.path);
        let text = theme.color(if gone {
            UiRole::TextMuted
        } else {
            UiRole::Text
        });
        let muted = theme.color(UiRole::TextMuted);
        let selection = theme.color(UiRole::SelectionLight);
        let button_hover = theme.color(UiRole::ListButtonHover);
        let radius = theme.metric(Metric::CornerRadius);
        let small = theme.ui_size() - px(1.0);
        let mut avatar = recent_projects::avatar(&p.name, p.color);
        if gone {
            // `IconUtil.desaturate`: the lightness of each colour.
            avatar.start = avatar.start.grayscale();
            avatar.end = avatar.end.grayscale();
        }
        let shown_path = project::display_path(&p.path);
        let tooltip: SharedString = if gone {
            format!("{shown_path} {UNAVAILABLE}").into()
        } else {
            shown_path.clone().into()
        };
        let path = p.path.clone();
        let name = p.name.clone();
        let open = {
            let path = path.to_string_lossy().into_owned();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.dispatch_action(Box::new(open_action(path.clone())), cx)
            }
        };
        let pressed = selected && self.menu_from_button && self.menu.is_some();
        let button = selected.then(|| {
            let path = path.clone();
            let (icon, hover_icon, size) = if gone {
                (
                    IconName::RECENT_PROJECT_REMOVE,
                    IconName::RECENT_PROJECT_REMOVE_HOVER,
                    REMOVE_ICON,
                )
            } else {
                (IconName::MORE_VERTICAL, IconName::MORE_VERTICAL, 16.0)
            };
            div()
                .id(("welcome-project-button", ix))
                .debug_selector(|| "welcome-project-button".into())
                .group("welcome-project-button")
                .absolute()
                .right(px(ROW_BUTTON_RIGHT))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .child(
                    div()
                        .size(px(ROW_BUTTON))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(ROW_BUTTON_RADIUS))
                        .when(pressed, |d| d.bg(button_hover))
                        .hover(move |s| s.bg(button_hover))
                        .child(
                            div()
                                .group_hover("welcome-project-button", |s| s.invisible())
                                .when(pressed, |d| d.invisible())
                                .child(Icon::new(icon).size(px(size))),
                        )
                        .child(
                            div()
                                .absolute()
                                .invisible()
                                .group_hover("welcome-project-button", |s| s.visible())
                                .when(pressed, |d| d.visible())
                                .child(Icon::new(hover_icon).size(px(size))),
                        ),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if gone {
                        let path = path.to_string_lossy().into_owned();
                        window.dispatch_action(Box::new(RemoveRecent { path }), cx);
                    } else {
                        this.open_menu(path.clone(), e.position(), true, window, cx);
                    }
                }))
        });
        let branch = branch.map(|branch| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(BRANCH_ICON_GAP))
                .h(px(ROW_BRANCH_LINE))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(small)
                .text_color(muted)
                .child(
                    div()
                        .flex_none()
                        .child(Icon::new(IconName::VCS).size(px(BRANCH_ICON)).tint(muted)),
                )
                .child(branch)
        });
        let hover_path = path.clone();
        div()
            .id(("welcome-project", ix))
            .debug_selector(move || format!("welcome-project:{name}"))
            .relative()
            .flex_none()
            .mx(px(ROW_INSET))
            .p(px(ROW_BORDER))
            .flex()
            .flex_row()
            .items_start()
            .rounded(radius)
            .when(selected, |d| d.bg(selection))
            .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                // The pointer selects the row it is over and, leaving it,
                // clears the selection if it is still that row's (the
                // tree's `ProjectActionMouseListener`; not while the
                // row's menu is open).
                if !*on && this.pressed.as_ref() == Some(&hover_path) {
                    this.pressed = None;
                }
                if this.menu.is_some() {
                    return;
                }
                let now = on.then(|| hover_path.clone());
                if (*on || this.selected.as_ref() == Some(&hover_path)) && this.selected != now {
                    this.selected = now;
                    cx.notify();
                }
            }))
            // No tooltip over the row's menu, nor after a press.
            .when(
                self.menu.is_none() && self.pressed.as_ref() != Some(&p.path),
                |d| d.tooltip(Tooltip::text(tooltip, None)),
            )
            .on_mouse_down(MouseButton::Left, {
                let path = path.clone();
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.pressed = Some(path.clone());
                    cx.notify();
                })
            })
            .on_click(open)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.pressed = Some(path.clone());
                    this.open_menu(path.clone(), e.position, false, window, cx);
                }),
            )
            .child(
                div()
                    .flex_none()
                    .mt(px(ROW_ICON))
                    .ml(px(ROW_ICON))
                    .mr(px(ROW_ICON_GAP))
                    .child(avatar.render(AVATAR)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .p(px(ROW_TEXT))
                    .gap(px(ROW_LINE_GAP))
                    // Up to the button, else to the renderer's end (the
                    // tree's, 12 beyond the selection) less its border:
                    // past the selection's end.
                    .map(|d| {
                        if selected {
                            d.mr(px(ROW_BUTTON + ROW_BUTTON_RIGHT - ROW_BORDER))
                        } else {
                            d.mr(px(-ROW_INSET))
                        }
                    })
                    .child(
                        div()
                            .line_height(px(ROW_NAME_LINE))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(text)
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .line_height(px(ROW_PATH_LINE))
                            .text_size(small)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(muted)
                            .child(shown_path),
                    )
                    .children(branch),
            )
            .children(button)
            .pointing_hand()
            .into_any_element()
    }

    /// White (dark themes) or black at `dark` or `light` opacity.
    fn wash(dark: f32, light: f32, cx: &App) -> Hsla {
        if cx.theme().is_dark() {
            gpui::white().opacity(dark)
        } else {
            gpui::black().opacity(light)
        }
    }

    fn render_button(ix: usize, button: Button, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let text = if button.enabled {
            theme.color(UiRole::Text)
        } else {
            theme.color(UiRole::TextDisabled)
        };
        let label = button.label;
        div()
            .id(("welcome-projects-button", ix))
            .debug_selector(move || format!("welcome-projects-button:{label}"))
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(BUTTON_HEIGHT))
            .pl(px(BUTTON_ICON_X))
            .pr(px(12.0))
            .gap(px(BUTTON_ICON_GAP))
            .rounded(px(BUTTON_RADIUS))
            .bg(Self::wash(0.07, 0.043, cx))
            .text_color(text)
            .child(
                div()
                    .flex_none()
                    .when(!button.enabled, |d| {
                        d.opacity(ui::components::DISABLED_ALPHA)
                    })
                    .child(Icon::new(button.icon).size(px(16.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .when(button.enabled, |d| {
                d.hover(|s| s.bg(Self::wash(0.12, 0.085, cx)))
                    .active(|s| s.bg(Self::wash(0.17, 0.09, cx)))
                    .on_click(|_: &ClickEvent, window, cx| {
                        window.dispatch_action(Box::new(OpenFolder { path: None }), cx)
                    })
            })
            .when(!button.enabled, |d| {
                d.tooltip(Tooltip::text(NOT_AVAILABLE, None))
            })
            .into_any_element()
    }
}

impl Render for WelcomeProjects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let line = theme.color(UiRole::Divider);
        let muted = theme.color(UiRole::TextMuted);
        let rule = || div().flex_none().h(px(1.0)).w_full().bg(line);
        let search_focus = self.search.focus_handle(cx);
        let empty: SharedString = NO_RECENT.into();
        let projects = self.shown(cx);
        let folders = recent_projects::folders(cx);
        let rows: Vec<AnyElement> = projects
            .iter()
            .enumerate()
            .map(|(ix, p)| self.render_row(ix, p, &folders, cx))
            .collect();
        let menu = self
            .menu
            .as_ref()
            .map(|(menu, at, _)| ui::menu::overlay(menu, *at, window));
        div()
            .id("welcome-projects")
            .debug_selector(|| "welcome-projects".into())
            .key_context("WelcomeProjects")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::first))
            .on_action(cx.listener(Self::last))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::remove_selected))
            .on_action(cx.listener(Self::escape))
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .child(rule())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .pt(px(BUTTONS_TOP))
                    .px(px(BUTTONS_X))
                    .gap(px(BUTTON_GAP))
                    .children(
                        BUTTONS
                            .iter()
                            .enumerate()
                            .map(|(ix, b)| Self::render_button(ix, *b, cx)),
                    ),
            )
            .child(rule().mt(px(LINE_TOP)))
            .child(
                div()
                    .id("welcome-projects-search")
                    .debug_selector(|| "welcome-projects-search".into())
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(px(SEARCH_HEIGHT))
                    .pl(px(SEARCH_ICON_X))
                    .pr(px(BUTTONS_X))
                    .gap(px(SEARCH_ICON_GAP))
                    .on_click(move |_: &ClickEvent, window, _| window.focus(&search_focus))
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted)
                            .child(Icon::new(IconName::SEARCH_SMALL).size(px(16.0))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(editor::field_line_height_for(true, window, cx))
                            .overflow_hidden()
                            .child(self.search.clone()),
                    ),
            )
            .child(rule())
            .when(!rows.is_empty(), |d| {
                d.child(
                    div()
                        .id("welcome-projects-list")
                        .debug_selector(|| "welcome-projects-list".into())
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .pt(px(LIST_TOP))
                        .pb(px(LIST_BOTTOM))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .children(rows),
                )
            })
            // The empty list's text a third of the way down.
            .when(projects.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .items_center()
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_none()
                                .debug_selector(|| "welcome-projects-empty".into())
                                .text_color(muted)
                                .child(empty),
                        )
                        .child(div().flex_1())
                        .child(div().flex_1()),
                )
            })
            .children(menu)
    }
}
