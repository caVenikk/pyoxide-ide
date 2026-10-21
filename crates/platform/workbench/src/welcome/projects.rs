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
//! Metrics of the reference: the buttons 34 high, 8 apart, 20 from the
//! sides and 8 below the top, corners of 8, the icon 14 from the left and
//! 12 before the label (`DisclosureButton`, `DarculaDisclosureButtonUI`);
//! their background white at 7 % (dark themes) or black at 4 %, 5 % more
//! under the mouse. The line 27 below the last button, the search row 38
//! high.

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::{
    AnyElement, App, ClickEvent, Context, DismissEvent, Entity, Focusable as _, Hsla, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Subscription, Window, div,
    prelude::*, px,
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
    /// The folders of the recent projects that are gone (looked at off the
    /// UI thread whenever the list changes).
    gone: HashSet<PathBuf>,
    /// The row under the pointer.
    hovered: Option<PathBuf>,
    /// A row's menu and where it opened.
    menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
}

impl WelcomeProjects {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = editor::field::new("", Some(SEARCH.into()), true, window, cx);
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<recent_projects::Store>(|this, cx| this.check_folders(cx))
            .detach();
        let mut this = Self {
            search,
            gone: HashSet::new(),
            hovered: None,
            menu: None,
        };
        this.check_folders(cx);
        this
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
            "hovered": self.hovered.as_ref().map(|p| p.to_string_lossy().into_owned()),
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
        let hovered = self.hovered.as_ref() == Some(&p.path);
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
        let button = hovered.then(|| {
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
            .when(hovered, |d| d.bg(hover))
            .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                // Leaving a row clears the hover only if it is still that
                // row's.
                let now = on.then(|| hover_path.clone());
                if (*on || this.hovered.as_ref() == Some(&hover_path)) && this.hovered != now {
                    this.hovered = now;
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
