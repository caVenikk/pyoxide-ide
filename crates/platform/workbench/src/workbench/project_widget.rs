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
//! Choosing an open project shows its window, choosing a recent one opens
//! it in this window.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, Corner, DismissEvent, Entity, Pixels, Point, Subscription, Window, point,
    px,
};
use serde_json::{Value as Json, json};
use ui::avatar::Avatar;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};

use super::Workbench;
use crate::commands::OpenFolder;
use crate::recent_projects::{self, COLORS};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// The project widget's popup.
    Show => "project-widget.show", "Project Widget",
        "Opens the project widget's popup (Open…, Clone Repository…, the open and the recent projects); closes it when it is open.";
    /// Open a recent project.
    OpenRecent { path: String } => "project.open-recent", "Open Recent Project",
        "Shows the window of a project open in the application, else opens the folder as the project in this window.",
        r#"{"path": "/path/to/project"}"#;
    /// Describe the popup.
    State => "project-widget.state", "Project Widget State",
        "Returns whether the project widget's popup is open and its rows as JSON (labels, folders, avatars, headers, selection, filter).";
}

/// `action.OpenFile.text` of the reference (the widget's Open…).
pub const OPEN: &str = "Open\u{2026}";
/// `action.ProjectFromVersionControl.ProjectWidgetPopup.text`.
pub const CLONE: &str = "Clone Repository\u{2026}";
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

/// The popup's rows.
fn items(rows: &[ProjectRow], cx: &App) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::action(OPEN, OpenFolder { path: None }).icon(IconName::FOLDER),
        // No clone dialog yet.
        MenuItem::unavailable(CLONE).icon(IconName::VCS),
    ];
    let row = |r: &ProjectRow| {
        let path = r.path.to_string_lossy().into_owned();
        MenuItem::action(r.name.clone(), OpenRecent { path })
            .avatar(recent_projects::avatar(&r.name, r.color, cx))
            .comment(project::display_path(&r.path))
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
        self.open_folder(path, window, cx);
    }

    /// The popup is open (the widget shows its hover background, as a
    /// combo button whose popup is shown: `ExpandableComboAction`).
    pub(super) fn project_widget_open(&self) -> bool {
        self.project_widget.popup.is_some()
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
