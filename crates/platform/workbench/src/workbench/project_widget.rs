//! The project widget's popup (`ProjectToolbarWidgetAction` of IntelliJ
//! Platform Community, Apache-2.0): Open… and Clone Repository… (the
//! `ProjectWidget.Actions` group), then the projects open in the
//! application's windows under "Open Projects" (this window's among
//! them), then the other recent projects under "Recent Projects", each in
//! the order of the recent projects (the one opened last first). A
//! project's row shows its avatar, its name and its folder (`~` for the
//! home folder). The list scrolls beyond 30 rows of the list's row height
//! (`ListPopupImpl`'s visible row count) and filters as one types.
//!
//! A row shows the branch of the project's Git work tree below the folder
//! as the branches were last looked at (the reference's cache: the popup
//! does not wait for them, opening it looks again for the next time).
//!
//! Choosing an open project shows its window, choosing a recent one opens
//! it in this window; a folder that is gone asks "Reopen Project" whether
//! to take it off the list.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, Corner, DismissEvent, Entity, Hsla, Pixels, Point, Rgba,
    Subscription, Window, div, linear_color_stop, linear_gradient, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Controls, UiRole};
use ui::avatar::Avatar;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::Workbench;
use super::run_actions::PendingDialog;
use crate::commands::OpenFolder;
use crate::dialog::{Dialog, DialogButton};
use crate::recent_projects::{self, COLORS};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// The project widget's popup.
    Show => "project-widget.show", "Project Widget",
        "Opens the project widget's popup (Open…, Clone Repository…, the open and the recent projects); closes it when it is open.";
    /// Open a recent project.
    OpenRecent { path: String, welcome: Option<bool> } => "project.open-recent", "Open Recent Project",
        "Shows the window of a project open in the application, else opens the folder as the project in this window; a folder that is gone asks \"Reopen Project\" whether to take it off the list (`welcome`: the Welcome tab's question, Cancel or Remove From List; else the popup's, Remove From List or OK).",
        r#"{"path": "/path/to/project"}"#;
    /// Show a recent project's folder in the file manager.
    RevealRecent { path: String } => "project.reveal-recent", "Show Recent Project in File Manager",
        "Selects the folder of a recent project in the file manager (`WelcomeScreen.RevealIn`: Reveal in Finder, Show in Explorer).",
        r#"{"path": "/path/to/project"}"#;
    /// Copy a recent project's folder.
    CopyRecentPath { path: String } => "project.copy-recent-path", "Copy Path",
        "Copies the folder of a recent project to the clipboard (`WelcomeScreen.CopyProjectPath`).",
        r#"{"path": "/path/to/project"}"#;
    /// Take a project off the recent projects.
    RemoveRecent { path: String } => "project.remove-recent", "Remove from Recent Projects\u{2026}",
        "Asks, then takes a project off the recent projects (`WelcomeScreen.RemoveSelected`).",
        r#"{"path": "/path/to/project"}"#;
    /// Describe the popup.
    State => "project-widget.state", "Project Widget State",
        "Returns whether the project widget's popup is open and its rows as JSON (labels, folders, avatars, headers, selection, filter).";
}

/// `action.OpenFile.text` of the reference (the widget's Open…).
pub const OPEN: &str = "Open\u{2026}";
/// `action.ProjectFromVersionControl.ProjectWidgetPopup.text`.
pub const CLONE: &str = "Clone Repository\u{2026}";
/// `dialog.title.remove.recent.project`.
pub const REMOVE_TITLE: &str = "Remove Recent Project";
/// `dialog.title.reopen.project`.
pub const REOPEN_TITLE: &str = "Reopen Project";
/// `button.remove.from.list`.
pub const REMOVE_FROM_LIST: &str = "&Remove From List";
/// `RevealFileAction.getActionName()`: `action.RevealIn.name.mac`, else
/// `action.RevealIn.name.other` with the file manager's name.
#[cfg(target_os = "macos")]
pub const REVEAL: &str = "Reveal in Finder";
#[cfg(windows)]
pub const REVEAL: &str = "Show in Explorer";
#[cfg(not(any(target_os = "macos", windows)))]
pub const REVEAL: &str = "Show in File Manager";

/// `message.the.path.0.does.not.exist.maybe.on.remote`.
pub fn missing_folder_message(path: &Path) -> String {
    format!(
        "The path {} does not exist.\nIf it is on a removable or network drive, please make sure that the drive is connected.",
        native_path::simplified(path).display()
    )
}
/// `project.widget.open.projects`.
pub const OPEN_PROJECTS: &str = "Open Projects";
/// `project.widget.recent.projects`.
pub const RECENT_PROJECTS: &str = "Recent Projects";
/// Rows of the list's row height the popup shows before it scrolls
/// (`ListPopupImpl`'s visible row count).
const MAX_ROWS: usize = 30;
/// Where the popup opens: its top left corner this far left of the
/// widget's background and below it (`showUnderneathOf` the combo button,
/// whose background is inset in its bounds; measured on the reference).
const POPUP_LEFT: f32 = 5.0;
const POPUP_BELOW: f32 = 4.0;

/// The project's colour on the main toolbar (`ProjectWindowCustomizerService`,
/// on by default: `UISettings.differentiateProjects`): the colour blended
/// into the toolbar's (`saturation` 0.85) at the centre of the project's
/// avatar, fading to the toolbar's colour on both sides. New UI themes
/// (`IslandsGradientPainter`): from the window's left edge to 600 right of
/// the centre (`RecentProject.MainToolbarGradient.width`), fading downwards
/// to the toolbar's colour at 300 (`.height`). Other themes
/// (`ProjectWindowCustomizerService.paint`): 300 on each side
/// (`ide.colorful.toolbar.gradient.radius`).
const SATURATION: f32 = 0.85;
const NEW_UI_WIDTH: f32 = 600.0;
const NEW_UI_HEIGHT: f32 = 300.0;
const CLASSIC_RADIUS: f32 = 300.0;
/// The centre before the widget was laid out (`DEFAULT_GRADIENT_OFFSET`).
const DEFAULT_CENTER: f32 = 150.0;

/// The widget's place and its open popup.
#[derive(Default)]
pub(super) struct ProjectWidgetUi {
    /// Where the widget is (the popup opens below it).
    pub anchor: Rc<Cell<Bounds<Pixels>>>,
    popup: Option<(Entity<ContextMenu>, Subscription)>,
}

/// A project of the popup: open in a window or recent.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub path: PathBuf,
    pub name: String,
    pub color: u8,
    pub open: bool,
}

/// The rows of the popup's two lists: the open projects (in `open`), then
/// the others, each in the order of `recent`; an open project the list
/// does not have yet comes first.
pub fn project_rows(
    recent: &recent_projects::RecentProjects,
    open: &[(PathBuf, String)],
) -> Vec<ProjectRow> {
    let is_open = |path: &Path| open.iter().any(|(p, _)| p == path);
    let mut rows: Vec<ProjectRow> = open
        .iter()
        .filter(|(path, _)| recent.get(path).is_none())
        .map(|(path, name)| ProjectRow {
            path: path.clone(),
            name: name.clone(),
            color: fallback_color(path),
            open: true,
        })
        .collect();
    let listed = recent.projects().iter().map(|p| ProjectRow {
        path: p.path.clone(),
        name: p.name.clone(),
        color: p.color,
        open: is_open(&p.path),
    });
    let (open_rows, closed): (Vec<_>, Vec<_>) = listed.partition(|r| r.open);
    rows.extend(open_rows);
    rows.extend(closed);
    rows
}

/// A colour for a project the list does not have yet (its folder's hash).
fn fallback_color(path: &Path) -> u8 {
    let hash = path
        .to_string_lossy()
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    (hash % u32::from(COLORS)) as u8
}

/// `a` and `b` mixed in RGB, `ratio` of `b` (`ColorUtil.blendColorsInRgb`).
fn blend(a: Hsla, b: Hsla, ratio: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let mix = |x: f32, y: f32| x + (y - x) * ratio;
    Rgba {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
        a: 1.0,
    }
    .into()
}

/// The popup's rows.
fn items(rows: &[ProjectRow], cx: &App) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::action(OPEN, OpenFolder { path: None }).icon(IconName::FOLDER),
        // No clone dialog yet.
        MenuItem::unavailable(CLONE).icon(IconName::VCS),
    ];
    // The branches as last looked at: the popup does not wait for them
    // (`GitRecentProjectsBranchesService.getCurrentBranch` answers from
    // its cache).
    let folders = recent_projects::folders(cx);
    let row = |r: &ProjectRow| {
        let path = r.path.to_string_lossy().into_owned();
        let item = MenuItem::action(
            r.name.clone(),
            OpenRecent {
                path,
                welcome: None,
            },
        )
        .avatar(recent_projects::avatar(&r.name, r.color, cx))
        .comment(project::display_path(&r.path));
        match folders.branches.get(&r.path) {
            Some(branch) => item.branch(branch.clone()),
            None => item,
        }
    };
    let (open, recent): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r.open);
    if !open.is_empty() {
        items.push(MenuItem::header(OPEN_PROJECTS));
        items.extend(open.into_iter().map(row));
    }
    if !recent.is_empty() {
        items.push(MenuItem::header(RECENT_PROJECTS));
        items.extend(recent.into_iter().map(row));
    }
    items
}

impl Workbench {
    /// The avatar of the window's project, in its colour from the recent
    /// projects (the toolbar's project widget).
    pub(super) fn project_avatar(&self, project: &project::Project, cx: &App) -> Avatar {
        let color = recent_projects::color(project.root(), cx)
            .unwrap_or_else(|| fallback_color(project.root()));
        recent_projects::avatar(project.name(), color, cx)
    }

    /// The project's colour on the main toolbar, `height` high: layers
    /// drawn before the toolbar's widgets (see [`SATURATION`]).
    pub(super) fn toolbar_project_color(&self, height: Pixels, cx: &App) -> Vec<AnyElement> {
        let Some(project) = self.project(cx) else {
            return Vec::new();
        };
        let theme = cx.theme();
        let main = theme.color(UiRole::Toolbar);
        let index = recent_projects::color(project.root(), cx)
            .unwrap_or_else(|| fallback_color(project.root()));
        let color = recent_projects::toolbar_color(index, cx);
        let blended = blend(main, color, SATURATION * color.a);
        let anchor = self.project_widget.anchor.get();
        let center = if anchor.size.width > px(0.0) {
            f32::from(anchor.origin.x) + crate::chrome::PROJECT_AVATAR_CENTER
        } else {
            DEFAULT_CENTER
        };
        let new_ui = theme.controls() == Controls::New;
        let (left, right) = if new_ui {
            (0.0, NEW_UI_WIDTH)
        } else {
            ((center - CLASSIC_RADIUS).max(0.0), CLASSIC_RADIUS)
        };
        let band = |x: f32, width: f32| div().absolute().top_0().left(px(x)).w(px(width)).h(height);
        let mut layers = vec![
            band(left, center - left)
                .debug_selector(|| "toolbar-project-color".into())
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(main, 0.0),
                    linear_color_stop(blended, 1.0),
                ))
                .into_any_element(),
            band(center, right)
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(blended, 0.0),
                    linear_color_stop(main, 1.0),
                ))
                .into_any_element(),
        ];
        if new_ui {
            // Down to the toolbar's colour at 300: the toolbar shows the
            // top of it.
            let bottom = (f32::from(height) / NEW_UI_HEIGHT).min(1.0);
            layers.push(
                band(0.0, center + right)
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(main.opacity(0.0), 0.0),
                        linear_color_stop(main.opacity(bottom), 1.0),
                    ))
                    .into_any_element(),
            );
        }
        layers
    }

    /// The projects open in the application's windows: this window's and
    /// those of the other main windows.
    pub(super) fn open_projects(&self, window: &Window, cx: &App) -> Vec<(PathBuf, String)> {
        let this = window.window_handle().window_id();
        let mut open = Vec::new();
        for handle in cx.windows() {
            if handle.window_id() == this {
                if let Some(p) = self.project(cx) {
                    open.push((p.root().to_owned(), p.name().to_owned()));
                }
                continue;
            }
            let Some(other) = handle.downcast::<Workbench>() else {
                continue;
            };
            if let Ok(workbench) = other.read(cx)
                && let Some(p) = workbench.project(cx)
            {
                open.push((p.root().to_owned(), p.name().to_owned()));
            }
        }
        open
    }

    pub(super) fn show_project_widget(
        &mut self,
        _: &Show,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((menu, _)) = self.project_widget.popup.take() {
            menu.update(cx, |m, cx| m.dismiss(window, cx));
            cx.notify();
            return;
        }
        let rows = project_rows(&recent_projects::list(cx), &self.open_projects(window, cx));
        // For the next time (the reference's cache fills as it is asked).
        recent_projects::check_folders(cx);
        let menu = ContextMenu::popup(
            items(&rows, cx),
            PopupOptions {
                speed_search: true,
                max_rows: Some(MAX_ROWS),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .project_widget
                .popup
                .as_ref()
                .is_some_and(|(m, _)| *m == menu)
            {
                this.project_widget.popup = None;
                cx.notify();
            }
        });
        self.project_widget.popup = Some((menu, subscription));
        cx.notify();
    }

    /// `project.open-recent`: the window that has the project open comes
    /// to the front, else the folder opens as the project here.
    pub(super) fn open_recent_project(
        &mut self,
        action: &OpenRecent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = PathBuf::from(&action.path);
        if self.project(cx).is_some_and(|p| p.root() == path) {
            window.activate_window();
            return;
        }
        let this = window.window_handle().window_id();
        let other = cx.windows().into_iter().find(|handle| {
            handle.window_id() != this
                && handle
                    .downcast::<Workbench>()
                    .and_then(|w| w.read(cx).ok())
                    .and_then(|w| w.project(cx))
                    .is_some_and(|p| p.root() == path)
        });
        if let Some(other) = other {
            let _ = other.update(cx, |_, window, _| window.activate_window());
            return;
        }
        // `RecentProjectItem.openProject`, `ReopenProjectAction`: a folder
        // that is gone asks first (`open_folder` looks at it on this thread
        // too).
        if !path.is_dir() {
            self.ask_reopen_missing(path, action.welcome == Some(true), window, cx);
            return;
        }
        self.open_folder(path, window, cx);
    }

    /// "Reopen Project" for a folder that is gone (`showReopenDialog`):
    /// Remove From List takes it off the list. The Welcome tab asks with
    /// Cancel and Remove From List, the default (`showYesNoDialog`); the
    /// popup with Remove From List and OK, the default (`showDialog`, the
    /// buttons in the macOS order).
    fn ask_reopen_missing(
        &mut self,
        path: PathBuf,
        welcome: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = missing_folder_message(&path);
        let buttons = if welcome {
            vec![
                DialogButton {
                    id: "cancel",
                    label: "&Cancel".into(),
                },
                DialogButton {
                    id: "remove",
                    label: REMOVE_FROM_LIST.into(),
                },
            ]
        } else {
            vec![
                DialogButton {
                    id: "remove",
                    label: REMOVE_FROM_LIST.into(),
                },
                DialogButton {
                    id: "ok",
                    label: "OK".into(),
                },
            ]
        };
        let dialog = cx.new(|cx| {
            Dialog::new(REOPEN_TITLE, message, buttons, cx).with_icon(IconName::ERROR_DIALOG)
        });
        self.open_dialog(dialog, PendingDialog::ReopenMissing(path), window, cx);
    }

    /// `project.reveal-recent`: the folder selected in the file manager
    /// (`RevealFileAction.openFile`).
    pub(super) fn reveal_recent_project(
        &mut self,
        action: &RevealRecent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.reveal_path(Path::new(&action.path));
    }

    /// The popup is open (the widget shows its hover background, as a
    /// combo button whose popup is shown: `ExpandableComboAction`).
    pub(super) fn project_widget_open(&self) -> bool {
        self.project_widget.popup.is_some()
    }

    /// `project.copy-recent-path`.
    pub(super) fn copy_recent_path(
        &mut self,
        action: &CopyRecentPath,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = native_path::simplified(Path::new(&action.path));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            path.to_string_lossy().into_owned(),
        ));
    }

    /// `project.remove-recent`: "Remove Recent Project" asks first
    /// (`RemoveSelectedProjectsAction.removeItems`).
    pub(super) fn remove_recent_project(
        &mut self,
        action: &RemoveRecent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = PathBuf::from(&action.path);
        let name = recent_projects::list(cx)
            .get(&path)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| project::display_path(&path));
        let dialog = cx.new(|cx| {
            Dialog::new(
                REMOVE_TITLE,
                format!("Remove '{name}' from recent projects list?"),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "&Cancel".into(),
                    },
                    DialogButton {
                        id: "remove",
                        label: "&Remove".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::QUESTION_DIALOG)
        });
        self.open_dialog(dialog, PendingDialog::RemoveRecentProject(path), window, cx);
    }

    /// The open popup's rows, for `project-widget.state`.
    pub(crate) fn project_widget_state(&self, cx: &App) -> Json {
        match &self.project_widget.popup {
            Some((menu, _)) => json!({ "open": true, "popup": menu.read(cx).state_json() }),
            None => json!({ "open": false }),
        }
    }

    /// The popup below the widget.
    pub(super) fn render_project_widget_popup(&self, window: &Window) -> Option<gpui::AnyElement> {
        let (menu, _) = self.project_widget.popup.as_ref()?;
        let anchor = self.project_widget.anchor.get();
        let at: Point<Pixels> = point(
            anchor.origin.x - px(POPUP_LEFT),
            anchor.bottom() + px(POPUP_BELOW),
        );
        Some(ui::menu::overlay_at(menu, at, Corner::TopLeft, window))
    }
}

/// Registers the commands and `project-widget.state`.
pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register_query(
        cx,
        "project-widget.state",
        Rc::new(|_, window, cx| {
            let workbench = window.root::<Workbench>().flatten().ok_or_else(|| {
                ui::command::CommandError::new("not_available", "not a main window")
            })?;
            Ok(workbench.read(cx).project_widget_state(cx))
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_projects_come_first_in_the_order_of_the_list() {
        let mut recent = recent_projects::RecentProjects::default();
        for (i, name) in ["alpha", "beta", "gamma", "delta"].iter().enumerate() {
            let path = PathBuf::from(format!("/work/{name}"));
            recent.opened(&path, name, i as u64, 0, |_| false);
        }
        let open = vec![
            (PathBuf::from("/work/beta"), "beta".to_owned()),
            (PathBuf::from("/work/delta"), "delta".to_owned()),
            (PathBuf::from("/work/new"), "new".to_owned()),
        ];
        let rows = project_rows(&recent, &open);
        let names: Vec<(&str, bool)> = rows.iter().map(|r| (r.name.as_str(), r.open)).collect();
        assert_eq!(
            names,
            [
                ("new", true),
                ("delta", true),
                ("beta", true),
                ("gamma", false),
                ("alpha", false),
            ]
        );
        // A project keeps its colour from the list.
        let beta = rows.iter().find(|r| r.name == "beta").unwrap();
        assert_eq!(
            beta.color,
            recent.get(Path::new("/work/beta")).unwrap().color
        );
    }
}
