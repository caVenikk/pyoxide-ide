//! The Welcome screen's left panel, "Projects and Files" (the reference's
//! `WelcomeScreenLeftPanel` of `intellij.platform.ide.nonModalWelcomeScreen`,
//! IntelliJ Community, Apache-2.0): what the project tool window shows
//! while no project is open. A column of buttons (Open…, New…, Clone
//! Repository…; `createToolWindowWelcomeScreenVerticalToolbar`), a line,
//! the "Search projects" field, a line, and the recent projects
//! (`RecentProjectFilteringTree`; without any, "No recent projects" a
//! third of the way down, where `StatusText` puts it).
//!
//! A recent project's row (`RecentProjectComponent`): a border of 4, the
//! avatar 6 in from it (top and left) and 8 before the name panel, which
//! has 4 around it, the name and below it, 4 apart, the folder in the
//! comment colour. A click opens the project. The pointer over a row
//! shows a button of 22, 20 from the row's right end: a gear with the
//! row's menu (`WelcomeScreenRecentProjectActionGroup`, the actions there
//! are here: Open Selected, Copy Path, Remove from Recent Projects…), or
//! for a folder that is gone a cross that takes it off the list (after
//! asking); such a row is greyed. A right click opens the menu too. The
//! search field filters the rows by name and folder.
//!
//! The keyboard (`RecentProjectFilteringTree`, `FilteringTree`): one row is
//! selected, as the reference's tree has it: the row under the pointer (the
//! tree selects the row the pointer moves over and clears the selection
//! when it leaves), the first row at the start, else the one the keys
//! moved to; the selected row shows the gear. Up and Down move the
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
//! under the mouse. The line 27 below the last button, the search row 38
//! high.

use std::collections::HashSet;
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
use ui::theme::ThemeExt;

use crate::commands::OpenFolder;
use crate::recent_projects::{self, RecentProject};
use crate::workbench::project_widget::{CopyRecentPath, OpenRecent, RemoveRecent};

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
const LINE_TOP: f32 = 27.0;
const SEARCH_HEIGHT: f32 = 38.0;
const SEARCH_ICON_X: f32 = 26.0;
const SEARCH_ICON_GAP: f32 = 8.0;
/// A project's row (see the module's notes) and the selection's inset
/// from the panel's sides (the tree's, as in the project tree).
const ROW_BORDER: f32 = 4.0;
const ROW_ICON: f32 = 6.0;
const ROW_ICON_GAP: f32 = 8.0;
const ROW_TEXT: f32 = 4.0;
const ROW_LINE_GAP: f32 = 4.0;
const ROW_NAME_LINE: f32 = 17.0;
const ROW_PATH_LINE: f32 = 15.0;
const AVATAR: f32 = 20.0;
const ROW_BUTTON: f32 = 22.0;
const ROW_BUTTON_RIGHT: f32 = 20.0;
const ROW_INSET: f32 = 12.0;
/// `recent.project.unavailable`, after a gone folder in its tooltip.
pub const UNAVAILABLE: &str = "(unavailable)";
/// `action.presentation.OpenSelectedProjectsAction.text.open.selected`.
pub const OPEN_SELECTED: &str = "Open Selected";
/// `action.WelcomeScreen.CopyProjectPath.text`.
pub const COPY_PATH: &str = "Copy Path";
/// `action.WelcomeScreen.RemoveSelected.text`.
pub const REMOVE: &str = "Remove from Recent Projects\u{2026}";

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

pub struct WelcomeProjects {
    search: Entity<editor::Editor>,
    /// The list's focus (the project tool window's: the panel hands it
    /// over while it shows this list).
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The folders of the recent projects that are gone (looked at off the
    /// UI thread whenever the list changes).
    gone: HashSet<PathBuf>,
    /// The selected row: the one under the pointer, or the one the keys
    /// moved to (a folder that is no longer listed selects nothing).
    selected: Option<PathBuf>,
    /// The first row was selected (`selectLastOpenedProjectOrTheFirstInTree`).
    initially_selected: bool,
    /// The search field's text when the list was last filtered.
    searched: String,
    /// A row's menu and where it opened.
    menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
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
            this.check_folders(cx);
        })
        .detach();
        let mut this = Self {
            search,
            focus,
            scroll: ScrollHandle::new(),
            gone: HashSet::new(),
            selected: None,
            initially_selected: false,
            searched: String::new(),
            menu: None,
        };
        this.list_changed(cx);
        this.check_folders(cx);
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
            window.dispatch_action(Box::new(OpenRecent { path }), cx);
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

    /// Looks for the recent projects' folders off the UI thread.
    fn check_folders(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = recent_projects::list(cx)
            .projects()
            .iter()
            .map(|p| p.path.clone())
            .collect();
        let check = cx.background_spawn(async move {
            paths
                .into_iter()
                .filter(|p| !p.is_dir())
                .collect::<HashSet<_>>()
        });
        cx.spawn(async move |this, cx| {
            let gone = check.await;
            let _ = this.update(cx, |this, cx| {
                if this.gone != gone {
                    this.gone = gone;
                }
                cx.notify();
            });
        })
        .detach();
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
                "gone": self.gone.contains(&p.path),
            })).collect::<Vec<_>>(),
            "selected": self
                .selected_folder(cx)
                .map(|p| project::display_path(std::path::Path::new(&p))),
            "menu": self.menu.as_ref().map(|(m, _, _)| m.read(cx).state_json()),
            "empty_text": NO_RECENT,
        })
    }

    /// The row's menu (`WelcomeScreenRecentProjectActionGroup`, what works
    /// here) at `position`.
    fn open_menu(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = path.to_string_lossy().into_owned();
        let items = vec![
            MenuItem::action(OPEN_SELECTED, OpenRecent { path: path.clone() }),
            MenuItem::action(COPY_PATH, CopyRecentPath { path: path.clone() }),
            MenuItem::separator(),
            MenuItem::action(REMOVE, RemoveRecent { path }),
        ];
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, position, subscription));
        cx.notify();
    }

    /// A recent project's row.
    fn render_row(&self, ix: usize, p: &RecentProject, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let gone = self.gone.contains(&p.path);
        let selected = self.selected.as_ref() == Some(&p.path);
        let text = theme.color(if gone {
            UiRole::TextMuted
        } else {
            UiRole::Text
        });
        let muted = theme.color(UiRole::TextMuted);
        let hover = theme.color(UiRole::Hover);
        let button_hover = theme.color(UiRole::ListButtonHover);
        let radius = theme.metric(Metric::CornerRadius);
        let small = theme.ui_size() - px(1.0);
        let mut avatar = recent_projects::avatar(&p.name, p.color, cx);
        if gone {
            // `IconUtil.desaturate`.
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
                window.dispatch_action(Box::new(OpenRecent { path: path.clone() }), cx)
            }
        };
        let button = selected.then(|| {
            let path = path.clone();
            let icon = if gone {
                IconName::CLOSE_SMALL
            } else {
                IconName::SETTINGS_SMALL
            };
            div()
                .id(("welcome-project-button", ix))
                .debug_selector(|| "welcome-project-button".into())
                .absolute()
                .right(px(ROW_BUTTON_RIGHT))
                .top(px((52.0 - ROW_BUTTON) / 2.0))
                .size(px(ROW_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius)
                .hover(move |s| s.bg(button_hover))
                .child(Icon::new(icon).size(px(16.0)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if gone {
                        let path = path.to_string_lossy().into_owned();
                        window.dispatch_action(Box::new(RemoveRecent { path }), cx);
                    } else {
                        this.open_menu(path.clone(), e.position(), window, cx);
                    }
                }))
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
            .when(selected, |d| d.bg(hover))
            .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                // The pointer selects the row it is over and, leaving it,
                // clears the selection if it is still that row's (the
                // tree's `ProjectActionMouseListener`; not while the
                // row's menu is open).
                if this.menu.is_some() {
                    return;
                }
                let now = on.then(|| hover_path.clone());
                if (*on || this.selected.as_ref() == Some(&hover_path)) && this.selected != now {
                    this.selected = now;
                    cx.notify();
                }
            }))
            // No tooltip over the row's menu.
            .when(self.menu.is_none(), |d| {
                d.tooltip(Tooltip::text(tooltip, None))
            })
            .on_click(open)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.open_menu(path.clone(), e.position, window, cx);
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
                    .pr(px(ROW_BUTTON + ROW_BUTTON_RIGHT))
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
                    ),
            )
            .children(button)
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
        let rows: Vec<AnyElement> = projects
            .iter()
            .enumerate()
            .map(|(ix, p)| self.render_row(ix, p, cx))
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
                            .child(Icon::new(IconName::SEARCH).size(px(16.0))),
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
