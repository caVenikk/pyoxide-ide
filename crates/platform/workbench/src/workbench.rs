//! The main window's root view: toolbar, stripes, the project tool window,
//! editor tabs, the status bar, and the commands that work on them.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use code_intel_ui::{CodeIntelEvent, CodeIntelView};
use editor::find::FindEvent;
use editor::{Editor, EditorEvent, FindBar};
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Bounds, ClickEvent, Context, DispatchPhase, Div,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Render, ScrollWheelEvent,
    SharedString, Subscription, Task, Window, canvas, div, prelude::*, px,
};
use project::Project;
use serde_json::json;
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button, island};
use ui::icons::{Icon, IconName};
use ui::notification::Notification;
use ui::theme::ThemeExt;

use crate::app::Product;
use crate::background_tasks::{BackgroundTasks, ProcessesPopup};
use crate::chrome;
use crate::commands::*;
use crate::env;
use crate::file_types::FileTypes;
use crate::find_in_files::{self, FindInFiles, FindInFilesEvent, OpenContext};
use crate::notifications::{ClearAllNotifications, Notifications, NotificationsToolWindow, Notify};
use crate::project_files::ProjectFiles;
use crate::project_panel::{self, PanelEvent, ProjectPanel};
use crate::recent_files::{self, RecentFiles, RecentFilesEvent};
use crate::run::RunToolWindow;

use crate::search_everywhere::{self, SearchEverywhere, SearchEverywhereEvent, Tab as SeTab};
use crate::settings::SaveSettings;
use crate::tab_view::AnyTabView;

mod branches;
mod commit;
mod debug;
mod diff;
mod documentation_window;
mod file_events;
mod file_sync;
mod history;
pub(crate) mod project_widget;
mod remote;
mod run_actions;
mod splits;
pub(crate) mod tab_menu;
pub(crate) mod terminal_window;
mod tool_window_areas;
mod version_control;
mod views;
mod welcome;
mod window_actions;

use crate::tool_windows::Area;
pub use crate::tool_windows::{BOTTOM_WEIGHT, PROJECT_WIDTH, ToolWindow};
pub use tool_window_areas::MainLayout;
/// Space before the first editor tab.
const FIRST_TAB_OFFSET: f32 = 3.0;
/// Space between a tab's area and its rounded background.
const TAB_H_OFFSET: f32 = 4.0;
/// Inner padding of a tab before its icon.
const TAB_PADDING: f32 = 8.0;
/// Opacity of the titles of tabs that are not selected.
const INACTIVE_TAB_ALPHA: f32 = 0.75;
/// Diameter of the dot that marks a modified file in its tab.
const MODIFIED_DOT: f32 = 6.0;
/// How long after a change a file that saves itself is written.
const AUTOSAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
/// The dot on a pinned tab's pin (`BadgeDotProvider(0.7, 0.2)` with a
/// radius of 3 on the 16 px icon): its diameter and top left corner.
const PIN_DOT: f32 = 6.0;
const PIN_DOT_X: f32 = 8.2;
const PIN_DOT_Y: f32 = 0.2;
/// How long a notice stays in the status bar (about as long as the
/// reference's balloons stay).
const NOTICE_TIME: std::time::Duration = std::time::Duration::from_secs(10);
/// The notice when Go to Declaration finds an object nothing can open.
const NO_OBJECT_HANDLER: &str = "Cannot open this declaration";
/// Between the run widget and the toolbar's buttons right of it: the run
/// widget's toolbar ends 16 after its buttons, the main toolbar's groups
/// are 10 apart, its buttons have 5 on both sides (`RedesignedRunWidget`
/// insets, `MainToolbar` layout gap, `MainToolbar.Icon.insets`).
const TOOLBAR_RUN_GAP: f32 = 31.0;
/// Between Search Everywhere and the gear, and after the gear.
const TOOLBAR_BUTTON_GAP: f32 = 10.0;
const TOOLBAR_END: f32 = 5.0;
/// Width of the Go to Line popup and its distance below the toolbar.
const GO_TO_LINE_WIDTH: f32 = 320.0;
const GO_TO_LINE_TOP: f32 = 80.0;

/// `line[:column]` as typed in the Go to Line field (one-based).
pub fn parse_line_column(text: &str) -> Option<(usize, Option<usize>)> {
    let text = text.trim();
    let (line, column) = match text.split_once([':', ',']) {
        Some((l, c)) => (l.trim(), Some(c.trim())),
        None => (text, None),
    };
    let line = line.parse::<usize>().ok().filter(|l| *l > 0)?;
    let column = match column {
        Some(c) if !c.is_empty() => Some(c.parse::<usize>().ok().filter(|c| *c > 0)?),
        _ => None,
    };
    Some((line, column))
}

/// Puts a file first in a list of recent files.
fn remember_file(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_owned());
    list.truncate(find_in_files::RECENT_FILES);
}

/// Text of the empty editor area (the reference IDE's editor empty text):
/// font size, line height (1.8 times the 20 px height of the font) and the
/// first baseline at 3/8 of the area's height.
const EMPTY_TEXT_SIZE: f32 = 16.0;
const EMPTY_TEXT_LINE_HEIGHT: f32 = 36.0;
const EMPTY_TEXT_BASELINE: f32 = 0.375;
/// Distance from the top of a line of that text to its baseline.
const EMPTY_TEXT_LINE_BASELINE: f32 = 24.0;

/// Reads and decodes a file into a buffer (blocking; run off the UI
/// thread). A file that does not decode cleanly opens read-only.
fn read_buffer(path: &Path) -> Result<text::Buffer, project::LoadError> {
    project::read_document(path).map(|loaded| {
        let read_only = loaded.read_only || loaded.decoded.lossy;
        let mut buffer = text::Buffer::from_decoded(loaded.decoded);
        buffer.set_read_only(read_only);
        buffer
    })
}

/// A path dropped onto the editor area, checked and read off the UI thread.
enum Dropped {
    Folder(PathBuf),
    /// A file and its contents; `None` when its tab is open already.
    File(PathBuf, Option<Result<text::Buffer, project::LoadError>>),
}

/// What happens in the main window that a product may follow. More kinds
/// of events may come.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WorkbenchEvent {
    /// A folder was opened as the project.
    ProjectOpened(PathBuf),
    /// Another file became the active editor's, or no file is active.
    ActiveFileChanged(Option<PathBuf>),
}

/// What an editor tab shows.
enum TabContent {
    /// A file in its editor, with the editor's find bar (shown while it is
    /// open).
    File {
        path: PathBuf,
        editor: Entity<Editor>,
        find: Entity<FindBar>,
    },
    /// A view that is no file's editor (a diff, [`crate::tab_view`]).
    View(Box<dyn AnyTabView>),
}

/// An editor tab.
struct Tab {
    content: TabContent,
    /// Pinned (`tab_menu.rs`).
    pinned: bool,
    /// The tab group (split window) the tab is in (`splits.rs`).
    group: crate::split_tree::GroupId,
    /// A second view of a file whose first view is in another group: its
    /// text follows the first view's (`splits.rs`).
    mirror: bool,
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    /// The file of a file's tab.
    fn path(&self) -> Option<&PathBuf> {
        match &self.content {
            TabContent::File { path, .. } => Some(path),
            TabContent::View(_) => None,
        }
    }

    fn editor(&self) -> Option<&Entity<Editor>> {
        match &self.content {
            TabContent::File { editor, .. } => Some(editor),
            TabContent::View(_) => None,
        }
    }

    fn find(&self) -> Option<&Entity<FindBar>> {
        match &self.content {
            TabContent::File { find, .. } => Some(find),
            TabContent::View(_) => None,
        }
    }

    fn view(&self) -> Option<&dyn AnyTabView> {
        match &self.content {
            TabContent::File { .. } => None,
            TabContent::View(view) => Some(view.as_ref()),
        }
    }

    /// The tab is `path`'s editor.
    fn is_file(&self, path: &Path) -> bool {
        self.path().is_some_and(|p| p == path)
    }

    /// What gets the focus when the tab is selected.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            TabContent::File { editor, .. } => editor.focus_handle(cx),
            TabContent::View(view) => view.focus_handle(cx),
        }
    }
}

/// The main window: toolbar, stripes, project tool window, editor area,
/// status bar.
pub struct Workbench {
    product: Rc<Product>,
    file_types: Rc<FileTypes>,
    focus: FocusHandle,
    /// The tool windows and the areas that show them.
    tool_windows: crate::tool_windows::ToolWindows,
    first_frame_reported: bool,
    panel: Entity<ProjectPanel>,
    /// The tabs of all groups; a group's tabs are in the order of this list.
    tabs: Vec<Tab>,
    /// The selected tab of the focused group.
    active: Option<usize>,
    /// The editor area's split windows (`splits.rs`).
    splits: splits::Splits,
    /// The bounds of each tab as last drawn (a tab's menu opened from a
    /// command opens below it).
    tab_bounds: Rc<std::cell::RefCell<Vec<Bounds<Pixels>>>>,
    /// The context menu of a tab while open (`tab_menu.rs`).
    tab_menu: Option<tab_menu::TabMenu>,
    /// Background reads and writes in flight.
    pending: usize,
    /// Last error or notice, shown in the status bar.
    message: Option<SharedString>,
    /// A notice shown in the status bar for a while (in place of the
    /// reference's balloons, until there are notifications).
    notice: Option<SharedString>,
    notice_timer: Task<()>,
    /// The Go to Line field while it is open, and its blur subscription.
    go_to_line: Option<(Entity<Editor>, Subscription)>,
    /// Resizing a tool window area.
    resizing: Option<tool_window_areas::Resizing>,
    save_settings: SaveSettings,
    /// The last keyboard or mouse input or edit (executor clock).
    last_input: Option<Instant>,
    /// Files with an autosave timer running ([`FileDecoration::autosave`]).
    autosave_pending: std::collections::HashSet<PathBuf>,
    /// A save after the idle time is scheduled.
    idle_save_pending: bool,
    /// The Find in Files popup (shown while it is open).
    find_in_files: Entity<FindInFiles>,
    /// Search Everywhere and Go to File (shown while open).
    search_everywhere: Entity<SearchEverywhere>,
    /// The project's file name index.
    project_files: Entity<ProjectFiles>,
    /// Recent Files (shown while open).
    recent_files: Entity<RecentFiles>,
    /// Where the focus was when the popup opened; Escape returns it there.
    return_focus: Option<FocusHandle>,
    /// A file being opened goes to this point once it is loaded.
    pending_go_to: Option<(PathBuf, text::Point)>,
    /// A file being opened gives the focus back to this element once it
    /// is loaded (the debugger shows a line without taking the focus).
    pub(crate) focus_after_open: Option<(PathBuf, FocusHandle)>,
    /// A file being read that opens in a given group, not the focused one
    /// (Open in Right Split).
    pub(crate) open_target: Option<(PathBuf, splits::OpenTarget)>,
    /// Files shown in editors and files edited, most recent first (the
    /// Recently Viewed and Recently Changed scopes of Find in Files).
    recently_viewed: Vec<PathBuf>,
    recently_changed: Vec<PathBuf>,
    /// The Run tool window (below the editor while visible).
    run_window: Entity<RunToolWindow>,
    /// The product's status bar widgets.
    status_widgets: Vec<AnyView>,
    /// The product's views above the editor of a file tab.
    editor_headers: Vec<AnyView>,
    /// The active file last reported with [`WorkbenchEvent::ActiveFileChanged`].
    reported_active_file: Option<PathBuf>,
    run_settings: run_actions::RunSettings,
    /// A popup of the run widget or a Run tab while open.
    run_menu: Option<run_actions::OpenMenu>,
    /// A modal dialog while open.
    dialog: Option<run_actions::OpenDialog>,
    /// Where the run widget's popups open.
    run_anchors: crate::run::widget::Anchors,
    /// Where the main toolbar's gear menu opens.
    pub(crate) gear: crate::settings::Gear,
    /// Code intelligence of the editor tabs, when the product has language
    /// services.
    code_intel: Option<Entity<CodeIntelView>>,
    /// Back and Forward (`history.rs`).
    navigation: history::Navigation,
    /// Changes of files on disk made outside the application.
    file_sync: file_sync::FileSync,
    /// Notification balloons.
    notifications: Entity<Notifications>,
    /// The background tasks in the status bar and the Processes popup.
    background_tasks: Entity<BackgroundTasks>,
    processes_popup: Entity<ProcessesPopup>,
    /// The project's Git repository and its statuses.
    version_control: version_control::VersionControl,
    /// The VCS widget's popups and dialogs.
    branch_ui: branches::BranchUi,
    /// The project widget's popup.
    project_widget: project_widget::ProjectWidgetUi,
    /// Push, Update Project and the VCS Operations popup.
    remote_ui: remote::RemoteUi,
    /// Breakpoints and the debugger (`debug.rs`).
    debugger: Entity<debugger_ui::DebuggerView>,
    /// The Commit tool window and its dialogs.
    commit_ui: commit::CommitUi,
    /// The Terminal tool window (`terminal_window.rs`).
    terminal_ui: terminal_window::TerminalUi,
    /// The diff tabs (`diff.rs`).
    diff_tabs: diff::DiffTabs,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorkbenchEvent> for Workbench {}

impl Workbench {
    /// The product's name (window titles).
    pub(crate) fn product_name(&self) -> &'static str {
        self.product.name
    }

    pub fn new(product: Rc<Product>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let file_types = Rc::new(FileTypes::with_associations(
            &product.file_types,
            &product.file_associations,
        ));
        let panel = cx.new(|cx| {
            let mut panel = ProjectPanel::new(file_types.clone(), cx);
            panel.set_new_items(product.new_items.clone());
            panel.set_product_rules(&product);
            panel
        });
        let find_in_files = cx.new(|cx| FindInFiles::new(file_types.clone(), window, cx));
        let host = cx.entity().downgrade();
        find_in_files.update(cx, |p, cx| p.set_host(host.clone(), cx));
        let search_everywhere = cx.new(|cx| SearchEverywhere::new(file_types.clone(), window, cx));
        search_everywhere.update(cx, |p, cx| p.set_host(host, cx));
        let project_files = cx.new(|_| ProjectFiles::new());
        let recent_place = search_everywhere::settings(cx).recent_files;
        let recent_files =
            cx.new(|cx| RecentFiles::new(file_types.clone(), recent_place, window, cx));
        let run_provider = product.run.clone();
        let run_window = cx.new(|cx| RunToolWindow::new(run_provider, window, cx));
        // The completion measurement brings a service of its own.
        let bench_intel = match env::bench() {
            Some(env::Bench::Completion(latency)) => Some(code_intel_ui::bench::hook(
                std::time::Duration::from_millis(*latency),
                crate::bench::COMPLETION_ITEMS,
            )),
            _ => None,
        };
        let hooks = match bench_intel {
            Some(hook) if product.code_intel.is_empty() => vec![hook],
            _ => product.code_intel.clone(),
        };
        let code_intel = (!hooks.is_empty()).then(|| cx.new(|cx| CodeIntelView::new(hooks, cx)));
        let mut subscriptions = vec![
            cx.subscribe_in(&run_window, window, |this, _, event, window, cx| {
                this.run_window_event(event, window, cx)
            }),
            cx.observe(&run_window, |_, _, cx| cx.notify()),
            // Processes do not outlive the application.
            cx.on_app_quit(|this, cx| this.kill_runs(cx)),
            cx.subscribe_in(&find_in_files, window, |this, _, event, window, cx| {
                match event {
                    FindInFilesEvent::OpenAt {
                        path,
                        point,
                        document,
                    } => this.open_file_at(path.clone(), *point, document.clone(), window, cx),
                    FindInFilesEvent::Dismissed => match this.return_focus.take() {
                        Some(focus) => window.focus(&focus),
                        None => this.focus_editor(&FocusEditor, window, cx),
                    },
                    FindInFilesEvent::Message(message) => {
                        this.message = Some(message.clone().into())
                    }
                }
                cx.notify();
            }),
            cx.observe(&find_in_files, |_, _, cx| cx.notify()),
            cx.subscribe_in(&search_everywhere, window, |this, _, event, window, cx| {
                this.search_everywhere_event(event, window, cx)
            }),
            cx.observe(&search_everywhere, |_, _, cx| cx.notify()),
            cx.subscribe_in(&recent_files, window, |this, _, event, window, cx| {
                this.recent_files_event(event, window, cx)
            }),
            cx.observe(&recent_files, |_, _, cx| cx.notify()),
            // The index follows the project's rules; the popup follows the
            // index.
            cx.observe(&project_files, |this, files, cx| {
                let index = files.read(cx).index();
                this.search_everywhere
                    .update(cx, |p, cx| p.set_index(index, cx));
            }),
            cx.subscribe_in(
                &panel,
                window,
                |this, panel, event, window, cx| match event {
                    PanelEvent::OpenFile(path) => this.open_file(path.clone(), window, cx),
                    PanelEvent::FilesChanged(changes) => this.files_changed(changes, window, cx),
                    PanelEvent::ReloadFromDisk(paths) => this.synchronize_paths(paths, cx),
                    PanelEvent::RulesChanged => {
                        if let Some(filter) = panel.read(cx).filter().cloned() {
                            this.project_files
                                .update(cx, |files, cx| files.set_rules(filter, cx));
                        }
                    }
                },
            ),
            // Files are saved when the window loses the focus, as in the
            // reference IDE.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.save_settings.save_on_deactivation {
                    this.save_all(cx);
                }
                // And files changed meanwhile are looked at when it comes
                // back.
                if window.is_window_active() {
                    this.window_activated(cx);
                }
            }),
        ];
        // Code intelligence popups and key contexts are drawn by the root.
        if let Some(view) = &code_intel {
            subscriptions.push(cx.observe(view, |_, _, cx| cx.notify()));
            subscriptions.push(cx.subscribe_in(view, window, |this, _, event, window, cx| {
                this.code_intel_event(event, window, cx)
            }));
        }
        let (debugger, debug_subscriptions) =
            Self::new_debugger_view(&product, file_types.clone(), window, cx);
        subscriptions.extend(debug_subscriptions);
        let notifications = cx.new(|cx| Notifications::new(window, cx));
        let background_tasks = cx.new(|cx| BackgroundTasks::new(window, cx));
        let processes_popup = cx.new(|cx| ProcessesPopup::new(background_tasks.clone(), cx));
        let notifications_window =
            cx.new(|cx| NotificationsToolWindow::new(notifications.clone(), cx));
        let mut tool_windows =
            tool_window_areas::default_tool_windows(&product, &run_window, &notifications_window);
        let commit_ui = commit::CommitUi::new(file_types.clone(), &mut tool_windows, window, cx);
        let terminal_ui = terminal_window::TerminalUi::new(&mut tool_windows, window, cx);
        Self::register_debug_tool_window(&mut tool_windows, &debugger, cx);
        // Names are coloured by their version control status (#169).
        let version_control = version_control::VersionControl::default();
        let colors = version_control.markers.colors.clone();
        panel.update(cx, |p, _| p.set_file_colors(colors.clone()));
        recent_files.update(cx, |p, _| p.set_file_colors(colors.clone()));
        search_everywhere.update(cx, |p, _| p.set_file_colors(colors));
        Self {
            product,
            file_types,
            focus,
            tool_windows,
            first_frame_reported: false,
            panel,
            tabs: Vec::new(),
            active: None,
            splits: Default::default(),
            pending: 0,
            message: None,
            notice: None,
            notice_timer: Task::ready(()),
            go_to_line: None,
            resizing: None,
            save_settings: SaveSettings::default(),
            last_input: None,
            autosave_pending: std::collections::HashSet::new(),
            idle_save_pending: false,
            find_in_files,
            search_everywhere,
            project_files,
            recent_files,
            return_focus: None,
            pending_go_to: None,
            focus_after_open: None,
            open_target: None,
            recently_viewed: Vec::new(),
            recently_changed: Vec::new(),
            run_window,
            status_widgets: Vec::new(),
            editor_headers: Vec::new(),
            reported_active_file: None,
            run_settings: run_actions::RunSettings::default(),
            run_menu: None,
            dialog: None,
            run_anchors: crate::run::widget::Anchors::default(),
            gear: Default::default(),
            code_intel,
            navigation: Default::default(),
            file_sync: Default::default(),
            notifications,
            background_tasks,
            processes_popup,
            version_control,
            branch_ui: Default::default(),
            project_widget: Default::default(),
            remote_ui: Default::default(),
            debugger,
            commit_ui,
            terminal_ui,
            diff_tabs: Default::default(),
            tab_bounds: Default::default(),
            tab_menu: None,
            _subscriptions: subscriptions,
        }
    }

    /// Code intelligence of the window, when the product has language
    /// services.
    pub fn code_intel(&self) -> Option<&Entity<CodeIntelView>> {
        self.code_intel.as_ref()
    }

    fn code_intel_event(
        &mut self,
        event: &CodeIntelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // A popup filled by an answer: drawn now, not at the next
            // refresh (as the find bar's matches are).
            CodeIntelEvent::Redraw => crate::shot::draw_now(window),
            CodeIntelEvent::Notice(text) => self.show_notice(text.clone().into(), cx),
            CodeIntelEvent::OpenFile(path) => self.open_file(path.clone(), window, cx),
            CodeIntelEvent::OpenText { title, text } => {
                self.open_text(title.clone(), text.clone(), window, cx)
            }
            CodeIntelEvent::OpenLocation { path, at } => {
                self.open_file_at(path.clone(), *at, None, window, cx)
            }
            CodeIntelEvent::OpenObject(target) => {
                if !crate::object_targets::open(target, window, cx) {
                    self.show_notice(NO_OBJECT_HANDLER.into(), cx);
                }
            }
            CodeIntelEvent::ShowDocumentation => self.show_documentation_window(window, cx),
        }
    }

    /// A notice in the status bar for ten seconds.
    pub fn show_notice(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(text.clone());
        self.notice_timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_TIME).await;
            let _ = this.update(cx, |this, cx| {
                if this.notice.as_ref() == Some(&text) {
                    this.notice = None;
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    /// The notice shown in the status bar.
    pub fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }

    /// Shows a notification balloon; returns its id.
    pub fn notify(&mut self, notification: Notification, cx: &mut Context<Self>) -> u64 {
        self.notifications
            .update(cx, |n, cx| n.notify(notification, cx))
    }

    /// The window's balloons.
    pub fn notifications(&self) -> &Entity<Notifications> {
        &self.notifications
    }

    fn notify_action(&mut self, action: &Notify, _: &mut Window, cx: &mut Context<Self>) {
        self.notify(action.notification(), cx);
    }

    fn clear_notifications(
        &mut self,
        _: &ClearAllNotifications,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notifications.update(cx, |n, cx| n.clear_timeline(cx));
    }

    /// Opens a text that is no file (a server's error output) in a
    /// read-only tab titled `title`, or shows it in that tab again.
    pub fn open_text(
        &mut self,
        title: String,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = PathBuf::from(&title);
        let mut buffer = text::Buffer::new(&text);
        buffer.set_read_only(true);
        if let Some(i) = self.file_tab(&path)
            && let Some(editor) = self.tabs[i].editor().cloned()
        {
            editor.update(cx, |e, cx| {
                *e.buffer_mut() = buffer;
                e.buffer_reloaded(cx);
            });
            self.activate(i, window, cx);
            return;
        }
        self.add_tab(path, buffer, window, cx);
    }

    /// Creates the product's status bar widgets (once, after the workbench
    /// entity exists).
    pub(crate) fn create_status_widgets(
        this: &Entity<Self>,
        product: &crate::app::Product,
        window: &mut Window,
        cx: &mut App,
    ) {
        let weak = this.downgrade();
        let views: Vec<AnyView> = product
            .status_widgets
            .iter()
            .map(|w| (w.create)(weak.clone(), window, cx))
            .collect();
        this.update(cx, |this, cx| {
            this.status_widgets = views;
            cx.notify();
        });
    }

    /// Creates the product's views above the editor (once, after the
    /// workbench entity exists).
    pub(crate) fn create_editor_headers(
        this: &Entity<Self>,
        product: &crate::app::Product,
        window: &mut Window,
        cx: &mut App,
    ) {
        let weak = this.downgrade();
        let views: Vec<AnyView> = product
            .editor_headers
            .iter()
            .map(|h| (h.create)(weak.clone(), window, cx))
            .collect();
        this.update(cx, |this, cx| {
            this.editor_headers = views;
            cx.notify();
        });
    }

    /// Makes the tool windows of the product's modules (once, after the
    /// workbench entity exists).
    pub(crate) fn create_module_tool_windows(
        this: &Entity<Self>,
        product: &crate::app::Product,
        window: &mut Window,
        cx: &mut App,
    ) {
        let weak = this.downgrade();
        let stripe = f32::from(cx.theme().metric(themes::Metric::StripeWidth));
        let pane = f32::from(window.viewport_size().width) - 2.0 * stripe;
        for hook in &product.tool_windows {
            let content = (hook.create)(weak.clone(), window, cx);
            let spec = hook.spec.clone();
            let width = hook
                .width
                .filter(|_| spec.area.is_side())
                .map(|w| w.min(crate::tool_windows::BOTTOM_WEIGHT * pane));
            this.update(cx, |this, cx| {
                let area = spec.area;
                this.register_tool_window(spec, Some(content), cx);
                if let Some(width) = width {
                    this.tool_windows.set_size(area, width);
                }
            });
        }
    }

    /// The project folder, when a project is open.
    pub fn project_root(&self, cx: &App) -> Option<PathBuf> {
        self.project(cx).map(|p| p.root().to_owned())
    }

    /// The file of the active editor tab.
    pub fn active_file(&self, _: &App) -> Option<PathBuf> {
        self.active.and_then(|i| self.tabs[i].path().cloned())
    }

    /// The Run tool window.
    pub fn run_window(&self) -> &Entity<RunToolWindow> {
        &self.run_window
    }

    /// Reports a change of the active file to subscribers.
    fn report_active_file(&mut self, cx: &mut Context<Self>) {
        let active = self.active_file(cx);
        if active != self.reported_active_file {
            self.reported_active_file = active.clone();
            if let Some(view) = &self.code_intel {
                view.update(cx, |v, cx| v.active_file_changed(active.as_deref(), cx));
            }
            cx.emit(WorkbenchEvent::ActiveFileChanged(active));
        }
    }

    pub fn project_visible(&self) -> bool {
        self.is_visible(ToolWindow::Project)
    }

    /// Width of the left tool window area.
    pub fn project_width(&self) -> Pixels {
        self.tool_windows.width(Area::Left)
    }

    /// The Go to Line field, while it is open.
    pub fn go_to_line_field(&self) -> Option<&Entity<Editor>> {
        self.go_to_line.as_ref().map(|(field, _)| field)
    }

    /// The path shown in the middle of the toolbar: relative to the project,
    /// then the full path in brackets; a view's title (`Commit: main.py`).
    pub fn title_path(&self, cx: &App) -> Option<String> {
        if let Some(view) = self.active.and_then(|i| self.tabs[i].view()) {
            return view
                .title_in_toolbar(cx)
                .then(|| view.title(cx).to_string());
        }
        let path = self.active_editor()?.read(cx).path()?.clone();
        let full = path.display().to_string();
        let decoration = self.file_types.decoration(&path);
        let relative = decoration.name.unwrap_or_else(|| {
            self.project(cx)
                .and_then(|p| p.relative(&path))
                .map(|r| r.display().to_string())
                .unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
        });
        // A file the type decorates names what it belongs to instead of its
        // location (a query file, the data source it is attached to).
        let place = decoration.hint.unwrap_or(full);
        Some(format!("{relative} [{place}]"))
    }

    pub fn panel(&self) -> &Entity<ProjectPanel> {
        &self.panel
    }

    /// The editor of the active tab.
    pub fn active_editor(&self) -> Option<&Entity<Editor>> {
        self.active.and_then(|i| self.tabs[i].editor())
    }

    /// The find bar of the active tab's editor.
    pub fn active_find_bar(&self) -> Option<&Entity<FindBar>> {
        self.active.and_then(|i| self.tabs[i].find())
    }

    /// The editor of the tab of `path`, when the file is open.
    pub fn editor_for(&self, path: &Path) -> Option<Entity<Editor>> {
        self.views_of(path).into_iter().next()
    }

    /// The index of the tab of `path`'s editor: the one in the focused
    /// group, else the first view's.
    fn file_tab(&self, path: &Path) -> Option<usize> {
        let focused = self.splits.focused;
        // The advanced setting "When navigating to a file, prefer
        // selecting existing tab in inactive split pane"
        // (`editor.open.inactive.splitter`, on by default): a file that is
        // open in another group is not opened again, its tab is selected
        // (`FileEditorManagerImpl.findWindowInAllSplitters`: the focused
        // group first, then the groups in the order they were made).
        self.tabs
            .iter()
            .position(|t| t.is_file(path) && t.group == focused)
            .or_else(|| {
                (0..self.tabs.len())
                    .filter(|&i| self.tabs[i].is_file(path))
                    .min_by_key(|&i| self.tabs[i].group)
            })
    }

    /// The Find in Files popup (drawn while it is open).
    pub fn find_in_files(&self) -> &Entity<FindInFiles> {
        &self.find_in_files
    }

    /// Paths of the open files' tabs, in order.
    pub fn tab_paths(&self) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .filter(|t| !t.mirror)
            .filter_map(|t| t.path().cloned())
            .collect()
    }

    /// Background work is still running (reads, writes, tree listings).
    pub fn is_busy(&self, cx: &App) -> bool {
        self.pending > 0 || self.panel.read(cx).is_loading()
    }

    fn project(&self, cx: &App) -> Option<Project> {
        self.panel.read(cx).project().cloned()
    }

    /// The state reported by `workbench.state`.
    pub fn state(&self, cx: &App) -> serde_json::Value {
        let project = self.project(cx);
        let rel = |p: &Path| -> String {
            project
                .as_ref()
                .and_then(|pr| pr.relative(p))
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| p.display().to_string())
        };
        json!({
            "product": self.product.name,
            "project": project.as_ref().map(|p| json!({
                "name": p.name(),
                "root": p.root().display().to_string(),
            })),
            "tree": self.panel.read(cx).state_json(cx),
            "projects_and_files": self.panel.read(cx).welcome_state(cx),
            "tool_windows": self.tool_windows.state_json(),
            "tool_window_areas": self.tool_windows.areas_json(),
            "title_path": self.title_path(cx),
            "status_path": self.project(cx).map(|p| self.status_path(&p, cx).0),
            "go_to_line": self.go_to_line_field().map(|f| f.read(cx).text()),
            "tabs": self.tabs.iter().map(|t| match &t.content {
                TabContent::File { path, editor, .. } => {
                    let decoration = self.file_types.decoration(path);
                    let decoration_autosave = decoration.autosave;
                    let mut tab = json!({
                        "path": rel(path),
                        "modified": editor.read(cx).is_modified(),
                        "deleted": self.is_deleted_on_disk(path),
                    });
                    // Only a decorated tab (a query file attached to a data
                    // source) has these keys.
                    if let Some(hint) = decoration.hint {
                        tab["hint"] = json!(hint);
                        tab["icon"] = json!(self.file_types.icon(path).0);
                    }
                    // Only an autosaved tab has the key.
                    if decoration_autosave {
                        tab["autosave"] = json!(true);
                    }
                    // Only a pinned tab has the key.
                    if t.pinned {
                        tab["pinned"] = json!(true);
                    }
                    // Only a second view of a file has the key.
                    if t.mirror {
                        tab["mirror"] = json!(true);
                    }
                    // Only a coloured tab has the key.
                    if let Some(color) = self.vcs_file_colors().name_color(path, false, false) {
                        tab["color"] = json!(color.name());
                    }
                    tab
                }
                TabContent::View(view) => {
                    let mut tab = json!({
                        "title": view.title(cx).to_string(),
                        "preview": view.is_preview(cx),
                        "view": view.state(cx),
                    });
                    if t.pinned {
                        tab["pinned"] = json!(true);
                    }
                    tab
                }
            }).collect::<Vec<_>>(),
            "active_tab": self.active,
            "splits": self.splits_json(cx),
            "tab_menu": self.tab_menu_json(cx),
            "navigation": self.navigation.history.state_json(rel),
            "editor": self.active_editor().map(|e| e.read(cx).state_json()),
            "find": self.active_find_bar().map(|f| f.read(cx).state_json(cx)),
            "find_in_files": self.find_in_files.read(cx).is_open(),
            "search_everywhere": self.search_everywhere.read(cx).is_open(),
            "recent_files": self.recent_files.read(cx).is_open(),
            "dialog": self.dialog.as_ref().map(|d| d.dialog.read(cx).state_json())
                .or_else(|| self.panel.read(cx).dialog().map(|d| d.read(cx).state_json())),
            "file_index": self.project_files.read(cx).index().map(|i| i.len()),
            "busy": self.is_busy(cx),
            "message": self.message.as_ref().map(|m| m.to_string()),
            "notice": self.notice.as_ref().map(|m| m.to_string()),
            "notifications": self.notifications.read(cx).state_json(cx),
            "background_tasks": self.background_tasks.read(cx).state_json(cx),
            "theme": cx.theme().id(),
            "vcs": self.vcs_state_json(cx),
            "diff": self.diff_state_json(cx),
            "switch_popup": self.switch_popup_json(cx),
            "keymap": crate::keymap::active::state_json(cx),
        })
    }

    // ---- project and files ----------------------------------------------------

    /// Opens a folder as the project.
    pub fn open_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = native_path::canonicalize(&path).unwrap_or(path);
        if !path.is_dir() {
            self.message = Some(format!("{} is not a folder", path.display()).into());
            cx.notify();
            return;
        }
        self.save_all(cx);
        self.tabs.clear();
        self.reset_splits();
        self.diff_tabs_cleared(cx);
        self.active = None;
        self.clear_navigation();
        self.tool_windows.show(ToolWindow::Project);
        self.project_files.update(cx, |files, cx| files.clear(cx));
        self.sync_project_opened(&path, window, cx);
        self.panel.update(cx, |panel, cx| {
            panel.set_project(Project::new(path.clone()), cx)
        });
        window.focus(&self.panel.focus_handle(cx));
        self.report_active_file(cx);
        if let Some(view) = &self.code_intel {
            view.update(cx, |v, cx| v.project_opened(&path, cx));
        }
        self.run_project_opened(&path, cx);
        self.debug_project_opened(&path, window, cx);
        crate::recent_projects::opened(cx, &path, Project::new(path.clone()).name());
        cx.emit(WorkbenchEvent::ProjectOpened(path));
        cx.notify();
    }

    fn resolve(&self, path: &str, cx: &App) -> PathBuf {
        let p = Path::new(path);
        match self.project(cx) {
            Some(project) if p.is_relative() => native_path::join(project.root(), p),
            _ => p.to_owned(),
        }
    }

    /// Opens a file in a tab (or selects its tab); reading happens off the
    /// UI thread.
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_in(path, None, window, cx);
    }

    /// Opens a file in a tab of the focused group, or in `target`'s; a file
    /// that is open already has its tab selected, unless `target` says
    /// where it should show.
    pub(crate) fn open_file_in(
        &mut self,
        path: PathBuf,
        target: Option<splits::OpenTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Some(target) => {
                if self.show_in_target(&path, target, window, cx) {
                    return;
                }
                self.open_target = Some((path.clone(), target));
            }
            None => {
                if let Some(i) = self.file_tab(&path) {
                    self.activate(i, window, cx);
                    return;
                }
            }
        }
        // A diff that edits the file's document: the tab continues with it.
        if let Some(buffer) = self.diff_document(&path, cx) {
            self.add_tab(path, buffer, window, cx);
            return;
        }
        self.pending += 1;
        // Reading, decoding and building the rope all happen off the UI
        // thread.
        let read = cx.background_spawn({
            let path = path.clone();
            async move { read_buffer(&path) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending -= 1;
                match loaded {
                    Ok(buffer) => this.add_tab(path, buffer, window, cx),
                    Err(e) => {
                        this.message = Some(format!("Cannot open {}: {e}", path.display()).into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a file (or selects its tab) with the caret at `point`, its line
    /// in the middle of the view.
    pub fn open_file_at(
        &mut self,
        path: PathBuf,
        point: text::Point,
        document: Option<text::Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = self.file_tab(&path)
            && let Some(editor) = self.tabs[i].editor().cloned()
        {
            let left = self.current_place(cx);
            self.activate(i, window, cx);
            editor.update(cx, |e, cx| {
                e.go_to(point.line + 1, Some(point.column + 1), cx)
            });
            if let Some(left) = left {
                self.record_place(left, cx);
            }
            return;
        }
        self.pending_go_to = Some((path.clone(), point));
        // Edited in the Find in Files preview: the tab takes that document.
        match document {
            Some(buffer) => self.add_tab(path, buffer, window, cx),
            None => self.open_file(path, window, cx),
        }
    }

    /// Opens what was dropped onto the editor area, as the reference IDE
    /// does: files open in tabs in the order they were dropped (a file that
    /// is open already has its tab selected) and the last one is selected;
    /// a folder of the project is selected in the project tool window, a
    /// folder outside it is left alone. Without a project, a dropped folder
    /// opens as the project and the rest of the drop is left alone, as on
    /// the reference IDE's welcome screen. Paths are checked and files read
    /// off the UI thread.
    pub fn open_dropped(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let has_project = self.project(cx).is_some();
        let open = self.tab_paths();
        self.pending += 1;
        let read = cx.background_spawn(async move {
            // As files named on the command line.
            let paths: Vec<PathBuf> = paths
                .into_iter()
                .map(|p| native_path::canonicalize(&p).unwrap_or(p))
                .collect();
            if !has_project && let Some(folder) = paths.iter().find(|p| p.is_dir()) {
                return vec![Dropped::Folder(folder.clone())];
            }
            paths
                .into_iter()
                .map(|path| {
                    if path.is_dir() {
                        Dropped::Folder(path)
                    } else {
                        let buffer = (!open.contains(&path)).then(|| read_buffer(&path));
                        Dropped::File(path, buffer)
                    }
                })
                .collect()
        });
        cx.spawn_in(window, async move |this, cx| {
            let dropped = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending -= 1;
                let mut error = None;
                for item in dropped {
                    match item {
                        Dropped::File(path, Some(Ok(buffer))) => {
                            this.add_tab(path, buffer, window, cx)
                        }
                        Dropped::File(path, Some(Err(e))) => {
                            error = Some(format!("Cannot open {}: {e}", path.display()))
                        }
                        // Selects its tab (or reads it if it has been
                        // closed since).
                        Dropped::File(path, None) => this.open_file(path, window, cx),
                        Dropped::Folder(path) => match this.project(cx) {
                            None => this.open_folder(path, window, cx),
                            Some(project) if project.relative(&path).is_some() => {
                                this.tool_windows.show(ToolWindow::Project);
                                this.panel.update(cx, |panel, cx| panel.reveal(&path, cx));
                                window.focus(&this.panel.focus_handle(cx));
                            }
                            Some(_) => {}
                        },
                    }
                }
                if let Some(error) = error {
                    this.message = Some(error.into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The editor of a file: its path, file type and the soft wraps setting.
    fn new_file_editor(
        &self,
        path: &Path,
        buffer: text::Buffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let file_type = self.file_types.for_path(path).cloned();
        let soft_wraps = crate::editor_settings::soft_wraps(cx);
        cx.new(|cx| {
            let mut editor = Editor::new(buffer, window, cx);
            editor.set_path(Some(path.to_owned()));
            if let Some(t) = &file_type {
                editor.set_config(t.editor_config());
                if let Some(factory) = &t.highlighter {
                    editor.set_highlighter(Some(factory()), cx);
                }
            }
            crate::editor_settings::apply(&soft_wraps, &mut editor, path, cx);
            editor
        })
    }

    /// The find bar of a file's tab and the subscriptions the tab keeps:
    /// to its editor (input for the idle save, recently changed files,
    /// jumps, the other views of the file) and to its find bar.
    fn file_tab_parts(
        &mut self,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<FindBar>, Vec<Subscription>) {
        let subscription = cx.subscribe(editor, move |this, editor, event: &EditorEvent, cx| {
            this.sync_views(&editor, event, cx);
            if matches!(event, EditorEvent::Edited(_)) {
                // Edits count as input for the idle save (they may also come
                // from commands rather than keys).
                this.note_input(cx);
                // The editor's path follows renames.
                if let Some(path) = editor.read(cx).path().cloned() {
                    remember_file(&mut this.recently_changed, &path);
                    if this.file_types.decoration(&path).autosave {
                        this.schedule_autosave(path, cx);
                    }
                }
            }
            if let EditorEvent::Jumped { from, view_y } = event
                && let Some(path) = editor.read(cx).path().cloned()
            {
                this.editor_jumped(path, *from, *view_y, cx);
            }
            cx.notify()
        });
        let find = cx.new(|cx| FindBar::new(editor.clone(), window, cx));
        // The bar opens and closes inside the editor area; matches found in
        // the background are drawn at once, not at the next refresh.
        let find_subscription = cx.subscribe_in(&find, window, |_, _, _: &FindEvent, window, _| {
            crate::shot::draw_now(window)
        });
        let observe_find = cx.observe(&find, |_, _, cx| cx.notify());
        (find, vec![subscription, find_subscription, observe_find])
    }

    fn add_tab(
        &mut self,
        path: PathBuf,
        buffer: text::Buffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .open_target
            .take_if(|(p, _)| *p == path)
            .map(|(_, t)| t);
        match target {
            // The file may have been opened while it was read.
            Some(target) => {
                if self.show_in_target(&path, target, window, cx) {
                    return;
                }
            }
            None => {
                if let Some(i) = self.file_tab(&path) {
                    self.activate(i, window, cx);
                    return;
                }
            }
        }
        let group = match target {
            Some(target) => self.target_group(target),
            None => self.splits.focused,
        };
        let file_type = self.file_types.for_path(&path).cloned();
        let editor = self.new_file_editor(&path, buffer, window, cx);
        let (find, subscriptions) = self.file_tab_parts(&editor, window, cx);
        if let Some((_, point)) = self.pending_go_to.take_if(|(p, _)| *p == path) {
            editor.update(cx, |e, cx| {
                e.go_to(point.line + 1, Some(point.column + 1), cx)
            });
        }
        self.find_in_files
            .update(cx, |p, cx| p.tab_opened(&path, &editor, cx));
        if let Some(view) = &self.code_intel {
            let language = file_type.as_ref().and_then(|t| t.language_id);
            view.update(cx, |v, cx| {
                v.editor_opened(&path, language, &editor, window, cx)
            });
        }
        self.sync_tab_opened(&path, &editor, window, cx);
        self.vcs_tab_opened(&path, &editor, window, cx);
        self.debug_tab_opened(&editor, window, cx);
        self.diff_tab_opened(&path, &editor, cx);
        let focus_back = self
            .focus_after_open
            .take_if(|(p, _)| *p == path)
            .map(|(_, f)| f);
        let index = self.tabs.len();
        self.tabs.push(Tab {
            content: TabContent::File { path, editor, find },
            pinned: false,
            group,
            mirror: false,
            _subscriptions: subscriptions,
        });
        self.message = None;
        self.activate(index, window, cx);
        if let Some(focus) = focus_back {
            window.focus(&focus);
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        if let Some(previous) = self.active
            && previous != index
            && self.save_settings.save_on_tab_switch
        {
            self.save_tab(previous, cx);
        }
        self.tab_activated(self.active, index, cx);
        if self.active != Some(index) {
            self.vcs_close_popup(cx);
        }
        self.active = Some(index);
        self.note_selected(index);
        if let Some(path) = self.tabs[index].path().cloned() {
            remember_file(&mut self.recently_viewed, &path);
        }
        window.focus(&self.tabs[index].focus_handle(cx));
        self.report_active_file(cx);
        cx.notify();
    }

    pub fn save_settings(&self) -> SaveSettings {
        self.save_settings
    }

    pub fn set_save_settings(&mut self, settings: SaveSettings) {
        self.save_settings = settings;
    }

    /// Keyboard or mouse input (or an edit) happened: the idle save waits
    /// for the idle time from now. One timer runs at a time; when it fires
    /// early because of later input it sleeps for the rest of the time.
    pub fn note_input(&mut self, cx: &mut Context<Self>) {
        if !self.save_settings.save_on_idle {
            return;
        }
        self.last_input = Some(cx.background_executor().now());
        if self.idle_save_pending {
            return;
        }
        self.idle_save_pending = true;
        let idle = self.save_settings.idle();
        cx.spawn(async move |this, cx| {
            let mut wait = idle;
            loop {
                cx.background_executor().timer(wait).await;
                let next = this.update(cx, |this, cx| {
                    let now = cx.background_executor().now();
                    let since = this
                        .last_input
                        .map_or(idle, |t| now.saturating_duration_since(t));
                    if since >= idle || !this.save_settings.save_on_idle {
                        this.idle_save_pending = false;
                        if this.save_settings.save_on_idle {
                            this.save_all(cx);
                        }
                        None
                    } else {
                        Some(idle - since)
                    }
                });
                match next {
                    Ok(Some(rest)) => wait = rest,
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// Saves `path` shortly after its last change; changes made while the
    /// timer runs share the one save.
    fn schedule_autosave(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.autosave_pending.insert(path.clone()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.autosave_pending.remove(&path);
                if let Some(i) = this.file_tab(&path) {
                    this.save_tab(i, cx);
                }
            });
        })
        .detach();
    }

    /// Writes one tab's file if it is modified; the returned task ends
    /// when the file is written.
    fn save_tab_task(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Task<()>> {
        // A second view of a file is saved by the first.
        let index = if index < self.tabs.len() {
            self.saving_tab(index)
        } else {
            index
        };
        let tab = self.tabs.get(index)?;
        let editor = tab.editor()?.clone();
        let path = tab.path()?.clone();
        if !editor.read(cx).is_modified() {
            return None;
        }
        // Not while the file changed or went away on disk; the write
        // leaves a file changed meanwhile alone.
        let guard = self.sync_save_started(&path, &editor, cx)?;
        let (bytes, point) = editor.update(cx, |e, _| {
            let point = e.buffer_mut().begin_save();
            (e.buffer().encoded(), point)
        });
        // The other views hold the same text.
        let others: Vec<_> = self
            .views_of(&path)
            .into_iter()
            .filter(|e| *e != editor)
            .map(|e| {
                let point = e.update(cx, |e, _| e.buffer_mut().begin_save());
                (e, point)
            })
            .collect();
        self.pending += 1;
        let write = cx.background_spawn({
            let path = path.clone();
            async move { guard.write(&path, &bytes) }
        });
        Some(cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.pending -= 1;
                let Some(result) = this.sync_save_finished(&path, result, cx) else {
                    return cx.notify();
                };
                match result {
                    Ok(()) => {
                        editor.update(cx, |e, cx| {
                            e.buffer_mut().mark_saved_at(point);
                            cx.notify();
                        });
                        for (other, point) in others {
                            other.update(cx, |e, cx| {
                                e.buffer_mut().mark_saved_at(point);
                                cx.notify();
                            });
                        }
                        if let Some(view) = &this.code_intel {
                            view.update(cx, |v, cx| v.file_saved(&path, cx));
                        }
                    }
                    Err(e) => {
                        this.message = Some(format!("Cannot save {}: {e}", path.display()).into())
                    }
                }
                cx.notify();
            });
        }))
    }

    /// Writes one tab's file if it is modified.
    fn save_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(task) = self.save_tab_task(index, cx) {
            task.detach();
        }
    }

    /// Writes every modified file; the tasks end when they are written.
    fn save_all_tasks(&mut self, cx: &mut Context<Self>) -> Vec<Task<()>> {
        let tasks = (0..self.tabs.len())
            .filter_map(|i| self.save_tab_task(i, cx))
            .collect();
        self.find_in_files.update(cx, |p, cx| p.save_preview(cx));
        tasks
    }

    pub fn save_all(&mut self, cx: &mut Context<Self>) {
        for i in 0..self.tabs.len() {
            self.save_tab(i, cx);
        }
        // A file edited in the Find in Files preview without a tab.
        self.find_in_files.update(cx, |p, cx| p.save_preview(cx));
        // And in a diff.
        self.diff_save_documents(cx);
    }

    /// Closes a tab; a modified file is saved first (no prompt). A modified
    /// editor that is not saved automatically (its file is gone, or changed
    /// on disk) asks first (Close Document) and stays until the answer.
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || self.sync_close_asks(index, cx) {
            return;
        }
        self.save_tab(index, cx);
        self.tab_closing(cx);
        let tab = self.tabs.remove(index);
        match &tab.content {
            TabContent::File { path, editor, .. } => {
                // A second view of a file is not known to the services.
                if !tab.mirror {
                    self.sync_tab_closed(path, cx);
                }
                self.vcs_tab_closed(editor, cx);
                self.debug_tab_closed(editor, cx);
                if !tab.mirror {
                    if let Some(view) = &self.code_intel {
                        view.update(cx, |v, cx| v.editor_closed(path, cx));
                    }
                    self.diff_file_tab_closed(path, cx);
                }
            }
            TabContent::View(view) => self.view_tab_closed(view.entity_id(), cx),
        }
        self.promote_mirror(&tab, window, cx);
        self.tab_removed(index, tab.group, tab.key(), true, window, cx);
        self.report_active_file(cx);
        cx.notify();
    }

    // ---- commands -------------------------------------------------------------

    fn locate_file(&mut self, _: &LocateFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .active_editor()
            .and_then(|e| e.read(cx).path().cloned())
        else {
            return;
        };
        self.tool_windows.show(ToolWindow::Project);
        self.panel.update(cx, |panel, cx| panel.reveal(&path, cx));
        window.focus(&self.panel.focus_handle(cx));
        cx.notify();
    }

    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        match self.active {
            Some(a) => window.focus(&self.tabs[a].focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    fn show_go_to_line(&mut self, _: &ShowGoToLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let p = editor.read(cx).caret_point();
        let text = format!("{}:{}", p.line + 1, p.column + 1);
        let field = cx.new(|cx| {
            let mut field = Editor::new(text::Buffer::new(&text), window, cx);
            field.set_single_line(true);
            field.set_ui_font(true);
            let len = field.buffer().len_chars();
            field.set_selections(text::SelectionSet::single(text::Selection::new(0, len)), cx);
            field
        });
        let handle = field.focus_handle(cx);
        window.focus(&handle);
        let blur = cx.on_blur(&handle, window, |this, _, cx| {
            this.go_to_line = None;
            cx.notify();
        });
        self.go_to_line = Some((field, blur));
        cx.notify();
    }

    fn confirm_go_to_line(
        &mut self,
        _: &ConfirmGoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.go_to_line_field() else {
            return;
        };
        // Text that is not a line number keeps the field open.
        let Some((line, column)) = parse_line_column(&field.read(cx).text()) else {
            return;
        };
        self.go_to_line = None;
        let left = self.current_place(cx);
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |e, cx| e.go_to(line, column, cx));
        }
        if let Some(left) = left {
            self.record_place(left, cx);
        }
        self.focus_editor(&FocusEditor, window, cx);
        cx.notify();
    }

    fn cancel_go_to_line(
        &mut self,
        _: &CancelGoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_line = None;
        self.focus_editor(&FocusEditor, window, cx);
        cx.notify();
    }

    /// What Find in Files opens with: the project, the active editor's
    /// selection while it has the focus, its open find bar's search, the
    /// open editors, the recently viewed and changed files, and the folder
    /// selected in the project tool window while it has the focus.
    fn find_in_files_context(&self, window: &Window, cx: &App) -> OpenContext {
        let selection = self
            .active_editor()
            .filter(|e| e.focus_handle(cx).contains_focused(window, cx))
            .map(|e| {
                let e = e.read(cx);
                e.buffer().slice(e.selections().primary().range())
            });
        let find_bar_text = self
            .active_find_bar()
            .map(|b| b.read(cx))
            .filter(|b| b.is_open())
            .map(|b| b.query_field().read(cx).text());
        let root = self.project(cx).map(|p| p.root().to_owned());
        // A folder selected in the tree (a selected file gives its folder).
        let tree_folder = self
            .panel
            .focus_handle(cx)
            .contains_focused(window, cx)
            .then(|| self.panel.read(cx).selected_path())
            .flatten()
            .and_then(|(path, is_dir)| {
                if is_dir {
                    Some(path.to_path_buf())
                } else {
                    path.parent().map(Path::to_path_buf)
                }
            });
        OpenContext {
            root,
            // The tree's rules: folders marked in this session count at once.
            filter: self.panel.read(cx).filter().cloned(),
            selection,
            find_bar_text,
            editors: self
                .tabs
                .iter()
                .filter(|t| !t.mirror)
                .filter_map(|t| Some((t.path()?.clone(), t.editor()?.clone())))
                .collect(),
            current_file: self.active.and_then(|i| self.tabs[i].path().cloned()),
            recently_viewed: self.recently_viewed.clone(),
            recently_changed: self.recently_changed.clone(),
            tree_folder,
        }
    }

    fn remember_focus(&mut self, window: &Window, cx: &App) {
        if !self.find_in_files.read(cx).is_open()
            && !self.search_everywhere.read(cx).is_open()
            && !self.recent_files.read(cx).is_open()
        {
            self.return_focus = window.focused(cx);
        }
    }

    /// The Recent Files popup.
    pub fn recent_files(&self) -> &Entity<RecentFiles> {
        &self.recent_files
    }

    fn open_recent_files(
        &mut self,
        _: &recent_files::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.recent_files.read(cx).is_open();
        if !open {
            self.remember_focus(window, cx);
        }
        let mut viewed = self.recently_viewed.clone();
        for path in self.tab_paths() {
            if !viewed.contains(&path) {
                viewed.push(path);
            }
        }
        let current = self.active.and_then(|i| self.tabs[i].path().cloned());
        // The current file first.
        if let Some(c) = &current {
            viewed.retain(|p| p != c);
            viewed.insert(0, c.clone());
        }
        let shortcut =
            ui::keys::shortcut_in(&recent_files::Open, &self.focus, window).map(Into::into);
        let tool_windows = self.tool_window_items(window, cx);
        let context = recent_files::OpenContext {
            root: self.project(cx).map(|p| p.root().to_owned()),
            viewed,
            edited: self.recently_changed.clone(),
            current,
            tool_windows,
            shortcut,
        };
        self.recent_files
            .update(cx, |p, cx| p.show(context, window, cx));
        cx.notify();
    }

    fn recent_files_event(
        &mut self,
        event: &RecentFilesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RecentFilesEvent::OpenFile(path) => self.open_file(path.clone(), window, cx),
            RecentFilesEvent::ShowToolWindow(id) => {
                self.activate_tool_window(ToolWindow(id), window, cx);
            }
            RecentFilesEvent::Remove { path, edited } => {
                self.recently_viewed.retain(|p| p != path);
                if *edited {
                    self.recently_changed.retain(|p| p != path);
                }
                if let Some(i) = self.file_tab(path) {
                    self.close_tab(i, window, cx);
                    // The popup stays open with the focus.
                    window.focus(&self.recent_files.focus_handle(cx));
                }
            }
            RecentFilesEvent::GoToFile(text) => {
                let action = search_everywhere::Search {
                    text: text.clone(),
                    tab: Some("files".to_owned()),
                    non_project: None,
                };
                self.search_everywhere_for(&action, window, cx);
            }
            RecentFilesEvent::Dismissed => match self.return_focus.take() {
                Some(focus) => window.focus(&focus),
                None => self.focus_editor(&FocusEditor, window, cx),
            },
        }
        cx.notify();
    }

    /// The popup of Find in Files.
    pub fn search_everywhere(&self) -> &Entity<SearchEverywhere> {
        &self.search_everywhere
    }

    /// The project's file name index.
    pub fn project_files(&self) -> &Entity<ProjectFiles> {
        &self.project_files
    }

    /// What Search Everywhere opens with: the editor's selection while it
    /// has the focus, the index, the open and recent files, and the
    /// commands as they are available where the focus is.
    fn search_everywhere_context(
        &self,
        window: &Window,
        cx: &mut App,
    ) -> search_everywhere::OpenContext {
        let selection = self
            .active_editor()
            .filter(|e| e.focus_handle(cx).contains_focused(window, cx))
            .map(|e| {
                let e = e.read(cx);
                e.buffer().slice(e.selections().primary().range())
            });
        let mut recent = self.recently_viewed.clone();
        for path in self.tab_paths() {
            if !recent.contains(&path) {
                recent.push(path);
            }
        }
        let focus = window.focused(cx);
        let actions = search_everywhere::actions::collect(focus.as_ref(), window, cx);
        let overrides = self
            .tabs
            .iter()
            .filter(|t| !t.mirror)
            .filter_map(|t| Some((t.path()?, t.editor()?)))
            .filter(|(_, e)| e.read(cx).is_modified())
            .map(|(p, e)| (p.clone(), e.read(cx).buffer().rope().clone()))
            .collect();
        search_everywhere::OpenContext {
            root: self.project(cx).map(|p| p.root().to_owned()),
            index: self.project_files.read(cx).index(),
            filter: self.panel.read(cx).filter().cloned(),
            overrides,
            selection,
            open_files: self.tab_paths(),
            current_file: self.active.and_then(|i| self.tabs[i].path().cloned()),
            recent_files: recent,
            actions,
            source_roots: self.project_files.read(cx).source_roots(),
            navigation: self.product.navigation,
            home: dirs::home_dir(),
        }
    }

    fn show_search_everywhere(&mut self, tab: SeTab, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.search_everywhere.read(cx).is_open();
        let context = if open {
            search_everywhere::OpenContext::default()
        } else {
            self.remember_focus(window, cx);
            self.search_everywhere_context(window, cx)
        };
        self.search_everywhere
            .update(cx, |p, cx| p.show(tab, context, window, cx));
        cx.notify();
    }

    fn open_search_everywhere(
        &mut self,
        _: &search_everywhere::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search_everywhere(SeTab::All, window, cx);
    }

    fn go_to_file(
        &mut self,
        _: &search_everywhere::GoToFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search_everywhere(SeTab::Files, window, cx);
    }

    fn go_to_action(
        &mut self,
        _: &search_everywhere::GoToAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search_everywhere(SeTab::Actions, window, cx);
    }

    fn go_to_text(
        &mut self,
        _: &search_everywhere::GoToText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search_everywhere(SeTab::Text, window, cx);
    }

    fn search_everywhere_for(
        &mut self,
        action: &search_everywhere::Search,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = action
            .tab
            .as_deref()
            .and_then(SeTab::from_name)
            .unwrap_or(SeTab::All);
        let context = if self.search_everywhere.read(cx).is_open() {
            search_everywhere::OpenContext::default()
        } else {
            self.remember_focus(window, cx);
            self.search_everywhere_context(window, cx)
        };
        let text = action.text.clone();
        let non_project = action.non_project;
        self.search_everywhere.update(cx, |p, cx| {
            p.search_for(tab, &text, non_project, context, window, cx)
        });
        cx.notify();
    }

    fn search_everywhere_event(
        &mut self,
        event: &SearchEverywhereEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchEverywhereEvent::OpenFile { path, line, column } => match line {
                Some(line) => {
                    let point = text::Point {
                        line: *line,
                        column: column.unwrap_or(0),
                    };
                    self.open_file_at(path.clone(), point, None, window, cx)
                }
                None => self.open_file(path.clone(), window, cx),
            },
            SearchEverywhereEvent::RevealFolder(path) => {
                self.tool_windows.show(ToolWindow::Project);
                self.panel.update(cx, |panel, cx| panel.reveal(path, cx));
                window.focus(&self.panel.focus_handle(cx));
            }
            SearchEverywhereEvent::Run(id) => {
                if let Ok(action) = cx.build_action(id, Some(json!({}))) {
                    window.dispatch_action(action, cx);
                }
            }
            SearchEverywhereEvent::Dismissed => match self.return_focus.take() {
                Some(focus) => window.focus(&focus),
                None => self.focus_editor(&FocusEditor, window, cx),
            },
            SearchEverywhereEvent::WantNonProject => {
                self.project_files
                    .update(cx, |files, cx| files.want_non_project(cx));
            }
            SearchEverywhereEvent::FindInFiles(text) => {
                let action = find_in_files::Search {
                    text: text.clone(),
                    case_sensitive: None,
                    whole_words: None,
                    regex: None,
                    mask: None,
                    replace: None,
                    scope: None,
                    directory: None,
                    recursive: None,
                };
                self.search_in_files(&action, window, cx);
            }
        }
        cx.notify();
    }

    fn open_find_in_files(
        &mut self,
        _: &find_in_files::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.find_in_files_context(window, cx);
        self.remember_focus(window, cx);
        self.find_in_files
            .update(cx, |p, cx| p.open(false, context, window, cx));
        cx.notify();
    }

    fn open_replace_in_files(
        &mut self,
        _: &find_in_files::OpenReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.find_in_files_context(window, cx);
        self.remember_focus(window, cx);
        self.find_in_files
            .update(cx, |p, cx| p.open(true, context, window, cx));
        cx.notify();
    }

    fn search_in_files(
        &mut self,
        action: &find_in_files::Search,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.find_in_files_context(window, cx);
        self.remember_focus(window, cx);
        self.find_in_files
            .update(cx, |p, cx| p.search_for(action, context, window, cx));
        cx.notify();
    }

    fn set_theme(&mut self, action: &SetTheme, _: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = settings_ui::theme_switch::switch(&action.id, cx) {
            self.message = Some(e.into());
            cx.notify();
        }
    }

    fn open_folder_action(
        &mut self,
        action: &OpenFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = &action.path {
            self.open_folder(PathBuf::from(path), window, cx);
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.open_folder(path, window, cx));
            }
        })
        .detach();
    }

    fn open_file_action(&mut self, action: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = &action.path {
            let path = self.resolve(path, cx);
            self.open_file(path, window, cx);
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.open_file(path, window, cx));
            }
        })
        .detach();
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.active {
            self.sync_explicit_save();
            self.save_tab(i, cx);
        }
    }

    fn save_all_action(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_explicit_save();
        self.save_all(cx);
    }

    fn close_tab_action(&mut self, a: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = a.index.or(self.active) {
            self.close_tab(i, window, cx);
        }
    }

    fn select_tab(&mut self, action: &SelectTab, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(action.index, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(true, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(false, window, cx);
    }

    /// The next (previous) tab of the focused group, round.
    fn step_tab(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.active else {
            return;
        };
        let order = self.group_tabs(self.tabs[a].group);
        let Some(at) = order.iter().position(|&i| i == a) else {
            return;
        };
        let to = if next {
            (at + 1) % order.len()
        } else {
            (at + order.len() - 1) % order.len()
        };
        self.activate(order[to], window, cx);
    }

    // ---- drawing --------------------------------------------------------------

    /// The main toolbar: the project widget on the left, the active file in
    /// the middle, the run widget, Search Everywhere and the gear on the
    /// right, laid out by [`main_toolbar`] (texts shrink first, then the
    /// right group runs past the window's edge).
    fn render_toolbar(
        &self,
        window: &Window,
        run: &crate::run::widget::WidgetState,
        cx: &App,
    ) -> impl IntoElement {
        use crate::main_toolbar::{Cut, Measure, Widget};
        let theme = cx.theme();
        let height = theme.metric(Metric::ToolbarHeight);
        let button = theme.metric(Metric::ToolbarButtonSize);
        let left = if cfg!(target_os = "macos") {
            px(chrome::TRAFFIC_LIGHTS_INSET)
        } else {
            px(8.0)
        };
        let measure = Measure::new(window, theme.ui_family(), theme.ui_size());
        let project = self.project(cx);
        let name = project.as_ref().map(|p| p.name().to_owned());
        let label = measure.text(
            chrome::project_label(name.as_deref()),
            FontWeight::SEMIBOLD,
            Cut::End,
        );
        let avatar = project.as_ref().map(|p| self.project_avatar(p, cx));
        let anchor = self.project_widget.anchor.clone();
        let popup_open = self.project_widget_open();
        let project_widget = Widget::text(
            chrome::project_widget_chrome(name.is_some()),
            label,
            move |label| {
                chrome::project_widget(name.as_deref(), avatar.as_ref(), &label, cx)
                    // Highlighted while its popup is open.
                    .when(popup_open, |d| d.bg(cx.theme().color(UiRole::Hover)))
                    .relative()
                    .child(
                        canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    )
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(project_widget::Show), cx)
                    })
                    .into_any_element()
            },
        );
        // The active file's path, centred in the toolbar while it fits.
        let file_widget = self.title_path(cx).map(|text| {
            let view_icon = self
                .active
                .and_then(|i| self.tabs[i].view())
                .map(|v| v.icon(cx));
            let icon = self
                .active_editor()
                .and_then(|e| e.read(cx).path().map(|p| self.file_types.icon(p)))
                .or(view_icon)
                .unwrap_or(IconName::FILE_UNKNOWN);
            let text = measure.text(&text, FontWeight::NORMAL, Cut::End);
            Widget::text(chrome::file_path_widget_chrome(), text, move |text| {
                chrome::file_path_widget(icon, &text, cx).into_any_element()
            })
        });
        let run_selector = Widget::text(
            crate::run::widget::selector_chrome(run),
            crate::run::widget::selector_text(run, &measure),
            |text| {
                crate::run::widget::render_selector(run, &self.run_anchors, &text, cx)
                    .into_any_element()
            },
        );
        let run_buttons = Widget::fixed(
            px(crate::run::widget::buttons_width(cx)),
            crate::run::widget::render_buttons(run, &self.run_anchors, cx),
        );
        let search = icon_button("search", IconName::SEARCH, button, px(20.0), false, cx)
            .tooltip(Tooltip::text("Search Everywhere", None));
        let mut left_group = vec![project_widget];
        left_group.extend(self.vcs_toolbar_widget(&measure, cx));
        let groups = [
            left_group,
            file_widget.into_iter().collect(),
            vec![
                run_selector,
                run_buttons,
                Widget::space(TOOLBAR_RUN_GAP),
                Widget::fixed(button, search),
                Widget::space(TOOLBAR_BUTTON_GAP),
                Widget::fixed(button, self.render_gear(button, cx)),
                Widget::space(TOOLBAR_END),
            ],
        ];
        let width = window.viewport_size().width - left;
        let project_color = self.toolbar_project_color(height, cx);
        div()
            .id("toolbar")
            .debug_selector(|| "toolbar".into())
            .relative()
            .flex_none()
            .h(height)
            .bg(theme.color(UiRole::Toolbar))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .children(project_color)
            .children(crate::main_toolbar::render(left, width, groups))
    }

    fn render_project(&self, width: Pixels, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        // Without a project: the Welcome screen's "Projects and Files",
        // whose header has no buttons.
        let tree = self.panel.read(cx).project().is_some();
        let header_h = theme.metric(Metric::ToolWindowHeaderHeight);
        let action = |id: &'static str, icon: IconName, title: &'static str| {
            icon_button(id, icon, px(24.0), px(16.0), false, cx).tooltip(Tooltip::text(title, None))
        };
        let command = |action: Box<dyn gpui::Action>| {
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.dispatch_action(action.boxed_clone(), cx)
            }
        };
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(header_h)
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child(if tree {
                                "Project"
                            } else {
                                crate::welcome::projects::TITLE
                            }),
                    )
                    .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0))),
            )
            .child(div().flex_1());
        let header = if !tree {
            header
        } else {
            header
                .child(
                    action("project-locate", IconName::LOCATE, "Select Opened File")
                        .on_click(command(Box::new(LocateFile))),
                )
                .child(
                    action("project-expand", IconName::EXPAND_ALL, "Expand All")
                        .on_click(command(Box::new(project_panel::ExpandAll))),
                )
                .child(
                    action("project-collapse", IconName::COLLAPSE_ALL, "Collapse All")
                        .on_click(command(Box::new(project_panel::CollapseAll))),
                )
                .child(action(
                    "project-options",
                    IconName::MORE_VERTICAL,
                    "Options",
                ))
                .child(
                    action("project-hide", IconName::HIDE, "Hide")
                        .on_click(command(Box::new(HideToolWindow))),
                )
        };
        island(cx, UiRole::ToolWindow)
            .id("project-tool-window")
            .debug_selector(|| "project-tool-window".into())
            .flex_none()
            .w(width)
            .h_full()
            .child(header)
            .child(self.panel.clone())
    }

    /// Height of the space between the toolbar and the status bar, which
    /// the tool windows share with the editor.
    fn pane_height(&self, window: &Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        window.viewport_size().height
            - theme.metric(Metric::ToolbarHeight)
            - chrome::status_metrics(cx).0
    }

    fn render_go_to_line(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let field = self.go_to_line_field()?.clone();
        let theme = cx.theme();
        let top = theme.metric(Metric::ToolbarHeight) + px(GO_TO_LINE_TOP);
        let popup = div()
            .id("go-to-line")
            .debug_selector(|| "go-to-line".into())
            .key_context("GoToLine")
            .occlude()
            .on_action(cx.listener(Self::confirm_go_to_line))
            .on_action(cx.listener(Self::cancel_go_to_line))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .w(px(GO_TO_LINE_WIDTH))
            .p(px(12.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .child(div().child("Go to Line:Column"))
            .child(
                div()
                    .h(px(30.0))
                    .px(px(6.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(theme.color(UiRole::Focus))
                    .bg(theme.color(UiRole::Input))
                    .child(field),
            );
        Some(
            div()
                .absolute()
                .top(top)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(popup),
        )
    }

    /// The find bar's hint ("not found, press ... to search from the top")
    /// below the caret.
    fn render_find_hint(
        &self,
        window_height: Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let bar = self.active_find_bar()?.read(cx);
        let (hint, above) = (bar.hint()?.clone(), bar.hint_above());
        let caret = self.active_editor()?.read(cx).caret_bounds()?;
        let theme = cx.theme();
        Some(
            div()
                .id("find-hint")
                .debug_selector(|| "find-hint".into())
                .absolute()
                .left(caret.left())
                // Below the caret line; a backward search's hint above it.
                .map(|d| {
                    if above {
                        d.bottom(window_height - caret.top() + px(2.0))
                    } else {
                        d.top(caret.bottom() + px(2.0))
                    }
                })
                .px(px(8.0))
                .py(px(4.0))
                .rounded(theme.metric(Metric::CornerRadius))
                .border_1()
                .border_color(theme.color(UiRole::PopupBorder))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow(ui::components::popup_shadow())
                .whitespace_nowrap()
                .child(hint),
        )
    }

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let pinned = tab.pinned;
        // The tab whose context menu is open looks hovered
        // (`JBTabsImpl.isHoveredOrWithPopup`).
        let with_menu = self.tab_menu_index() == Some(index);
        let theme = cx.theme();
        let selected = self.is_selected_in_group(index);
        let pill_h = theme.metric(Metric::TabSelectionHeight);
        let radius = theme.metric(Metric::ButtonRadius);
        let hover = theme.color(UiRole::TabHover);
        // The selected tab of a group without the focus has no outline,
        // as the selected tab of a strip without the focus
        // (`EditorTabs.inactiveUnderlinedTabBackground`).
        let in_focused_group = tab.group == self.splits.focused;
        let active_bg = if in_focused_group {
            theme.color(UiRole::TabActive)
        } else {
            theme.color(UiRole::TabSelectedInactive)
        };
        let outline = if in_focused_group {
            theme.color(UiRole::TabUnderline)
        } else {
            theme.color(UiRole::TabUnderlineInactive)
        };
        let (title, icon, text, modified, tooltip) = match &tab.content {
            TabContent::File { path, editor, .. } => {
                // A file gone from disk whose editor keeps its changes;
                // else the colour of its version control status.
                let status = self.vcs_file_colors().color(path, false, cx);
                let text: Hsla = if self.is_deleted_on_disk(path) {
                    theme.color(UiRole::Danger)
                } else if selected {
                    status.unwrap_or(theme.color(UiRole::TabActiveText))
                } else {
                    status
                        .unwrap_or(theme.color(UiRole::TabInactiveText))
                        .opacity(INACTIVE_TAB_ALPHA)
                };
                let mut name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let decoration = self.file_types.decoration(path);
                let decoration_autosave = decoration.autosave;
                if let Some(shown) = decoration.name {
                    name = shown;
                }
                if let Some(hint) = decoration.hint {
                    name = format!("{name} [{hint}]");
                }
                let modified = editor.read(cx).is_modified() && !decoration_autosave;
                (
                    SharedString::from(name),
                    self.file_types.icon(path),
                    text,
                    modified,
                    None,
                )
            }
            // A preview tab's title is grey.
            TabContent::View(view) => {
                let text: Hsla = if view.is_preview(cx) {
                    theme.color(UiRole::TextMuted)
                } else if selected {
                    theme.color(UiRole::TabActiveText)
                } else {
                    theme
                        .color(UiRole::TabInactiveText)
                        .opacity(INACTIVE_TAB_ALPHA)
                };
                (view.title(cx), view.icon(cx), text, false, view.tooltip(cx))
            }
        };
        let dragged = splits::DraggedTab::new(
            tab.key(),
            title.clone(),
            icon,
            self.tab_bounds
                .borrow()
                .get(index)
                .map_or(px(0.0), |b| b.size.width),
        );
        let group: SharedString = format!("tab-{index}").into();
        // A modified file shows a dot where the close button is; hovering
        // the dot shows the close button (as the reference IDE's New UI tabs
        // do when modified tabs are marked).
        let close_group: SharedString = format!("tab-close-{index}").into();
        let dot = theme.color(UiRole::Accent);
        // A pinned tab shows the pin there, with the dot when modified; a
        // click unpins it (`CloseTab`, "ide.editor.tabs.interactive.pin.
        // button").
        let close_icon = div()
            .absolute()
            .size_full()
            .when(pinned, |d| {
                d.debug_selector(move || format!("tab-pin-{index}"))
            })
            .when(modified && !pinned, |d| {
                d.invisible()
                    .group_hover(close_group.clone(), |s| s.visible())
            })
            .child(Icon::new(if pinned {
                IconName::PIN
            } else {
                IconName::CLOSE_SMALL
            }))
            .when(pinned && modified, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(PIN_DOT_X))
                        .top(px(PIN_DOT_Y))
                        .size(px(PIN_DOT))
                        .rounded_full()
                        .bg(dot),
                )
            });
        let marker = (modified && !pinned).then(|| {
            div()
                .debug_selector(move || format!("tab-modified-{index}"))
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .group_hover(close_group.clone(), |s| s.invisible())
                .child(div().size(px(MODIFIED_DOT)).rounded_full().bg(dot))
        });
        let close = div()
            .id(("tab-close", index))
            .group(close_group.clone())
            .relative()
            .flex_none()
            .size(px(16.0))
            .rounded(px(3.0))
            .when(!selected && !modified && !pinned, |d| {
                d.invisible().group_hover(group.clone(), |s| s.visible())
            })
            .hover(move |s| s.bg(hover))
            .child(close_icon)
            .children(marker)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if this.is_tab_pinned(index) {
                    this.toggle_pin(index, cx);
                } else {
                    this.close_tab(index, window, cx);
                }
            }));
        // Where the tab is drawn, for its menu opened from a command.
        let tab_bounds = self.tab_bounds.clone();
        let measure = canvas(
            move |bounds, _, _| {
                let mut all = tab_bounds.borrow_mut();
                if all.len() <= index {
                    all.resize(index + 1, Bounds::default());
                }
                all[index] = bounds;
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let pill = div()
            .id(("tab", index))
            .debug_selector(move || format!("tab-{index}"))
            .group(group)
            .flex()
            .flex_row()
            .items_center()
            .h(pill_h)
            .pl(px(TAB_PADDING))
            .pr(px(4.0))
            .gap(px(6.0))
            .rounded(radius)
            .border_1()
            .relative()
            .when(selected, |d| d.bg(active_bg).border_color(outline))
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .hover(move |s| s.bg(hover))
                    .when(with_menu, |d| d.bg(hover))
            })
            .child(measure)
            .child(Icon::new(icon))
            .child(div().whitespace_nowrap().text_color(text).child(title))
            .child(close)
            .when_some(tooltip, |d, t| d.tooltip(Tooltip::text(t, None)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate(index, window, cx);
            }))
            // The tab can be dragged to another place or group
            // (`splits/tab_drag.rs`).
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| splits::TabChip::new(dragged))
            })
            // The tab's context menu where the pointer is; the active tab
            // stays.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_editor_tab_menu(index, e.position, window, cx);
                }),
            );
        div()
            .flex_none()
            .px(px(TAB_H_OFFSET))
            .child(pill)
            .into_any_element()
    }

    /// The lines of the empty editor area: the reference IDE's hints, in its
    /// order, for the actions Pyoxide has (the Navigation Bar is not there
    /// yet): Search Everywhere, Project View, Go to File, Recent Files. An
    /// action's hint shows its shortcut ("Double ⇧" for a double tap) and is
    /// left out when it has none; the Project View hint shows only while
    /// the project tool window is hidden. Without a project the first line
    /// says how to start. None while a file is open.
    pub fn empty_text(&self, window: &Window, cx: &App) -> Vec<(&'static str, Option<String>)> {
        let mut lines = Vec::new();
        if !self.tabs.is_empty() {
            return lines;
        }
        if self.project(cx).is_none() {
            lines.push(("Open a folder to start", None));
        }
        let keys = |command: &str| {
            if let Some(key) = crate::keymap::double_tap_for(command, cx) {
                return Some(crate::keymap::double_tap_text(
                    key,
                    cfg!(target_os = "macos"),
                ));
            }
            let action = cx.build_action(command, None).ok()?;
            ui::keys::shortcut_in(action.as_ref(), &self.focus, window)
        };
        let mut hint = |text: &'static str, command: &str| {
            if let Some(keys) = keys(command) {
                lines.push((text, Some(keys)));
            }
        };
        hint("Search Everywhere", "search-everywhere.open");
        if !self.project_visible() {
            hint("Project View", "workbench.toggle-project");
        }
        hint("Go to File", "search-everywhere.go-to-file");
        hint("Recent Files", "recent-files.open");
        lines.push(("Drop files here to open them", None));
        lines
    }

    /// The editor island. Files dropped anywhere on it (the tabs, the
    /// editor or the empty area), from the file manager or from the
    /// Project tree, open in tabs, as in the reference IDE (its editor
    /// takes the file list of either drag, `FileDropManager`). It takes the
    /// width the side areas leave, at least `min`.
    fn editor_island(&self, min: Pixels, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        island(cx, UiRole::EditorBackground)
            .id("editor-area")
            .debug_selector(|| "editor-area".into())
            .flex_1()
            .min_w(min)
            .h_full()
            .on_drop(
                cx.listener(|this, dropped: &gpui::ExternalPaths, window, cx| {
                    this.open_dropped(dropped.paths().to_vec(), window, cx)
                }),
            )
            .on_drop(
                cx.listener(|this, dragged: &project_panel::DraggedFiles, window, cx| {
                    this.open_dropped(dragged.paths.clone(), window, cx)
                }),
            )
    }

    /// The active tab's editor, with what code intelligence adds around
    /// it (its commands, and the key context of a popup over it).
    fn editor_container(&self, editor: &Entity<Editor>, cx: &App) -> Div {
        let container = div().flex_1().min_h(px(0.0)).child(editor.clone());
        let container = match &self.code_intel {
            Some(view) => code_intel_ui::editor_container(container, view, editor, cx),
            None => container,
        };
        self.vcs_editor_container(container, editor)
    }

    fn render_editor_area(
        &self,
        min: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.active.is_some() || !self.splits.is_single() {
            return self
                .editor_island(min, cx)
                .child(self.render_groups(cx))
                .into_any_element();
        }
        // As the reference IDE's text painter draws them: a 16 px label
        // font, lines 1.8 times the font height (20 px) apart, left-aligned
        // in a block centred across the area, the first baseline at 3/8 of
        // its height; a shortcut follows its action after a space (two in a
        // dark theme).
        let theme = cx.theme();
        let shortcut = gpui::HighlightStyle {
            color: Some(theme.color(UiRole::Shortcut)),
            ..Default::default()
        };
        let space = if theme.is_dark() { "  " } else { " " };
        let muted = theme.color(UiRole::TextMuted);
        let mut lines = Vec::new();
        for (i, (action, keys)) in self.empty_text(window, cx).into_iter().enumerate() {
            let mut text = action.to_owned();
            let mut highlights = Vec::new();
            if let Some(keys) = keys {
                text.push_str(space);
                highlights.push((text.len()..text.len() + keys.len(), shortcut));
                text.push_str(&keys);
            }
            lines.push(
                div()
                    .debug_selector(move || format!("empty-text-{i}"))
                    .whitespace_nowrap()
                    .child(gpui::StyledText::new(text).with_highlights(highlights)),
            );
        }
        self.editor_island(min, cx)
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(gpui::relative(EMPTY_TEXT_BASELINE))
                    .mt(-px(EMPTY_TEXT_LINE_BASELINE))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .text_size(px(EMPTY_TEXT_SIZE))
                            .line_height(px(EMPTY_TEXT_LINE_HEIGHT))
                            .text_color(muted)
                            .children(lines),
                    ),
            )
            .into_any_element()
    }

    /// The path the status bar shows: the project and the active file's
    /// place in it, or the file's own breadcrumb (a console), and the icon
    /// of the last part.
    fn status_path(&self, project: &Project, cx: &App) -> (Vec<String>, Option<IconName>) {
        let mut parts: Vec<String> = vec![project.name().to_owned()];
        let mut icon = None;
        if let Some(editor) = self.active_editor()
            && let Some(path) = editor.read(cx).path()
        {
            let decoration = self.file_types.decoration(path);
            if let Some(crumbs) = decoration.breadcrumb.filter(|c| !c.is_empty()) {
                parts = crumbs;
                icon = Some(
                    decoration
                        .icon
                        .unwrap_or_else(|| self.file_types.icon(path)),
                );
            } else if let Some(rel) = project.relative(path) {
                parts.extend(rel.iter().map(|c| c.to_string_lossy().into_owned()));
                icon = Some(self.file_types.icon(path));
            }
        }
        (parts, icon)
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (height, text_size) = chrome::status_metrics(cx);
        // Nothing shrinks (`IdeStatusBarImpl`: a `BorderLayout` with the
        // breadcrumbs in the centre and the widgets east): the breadcrumbs
        // are cut where the widgets begin; with no room left the widgets
        // keep their width and run past the window's left edge.
        let mut left = div()
            .debug_selector(|| "status-left".into())
            .flex()
            .flex_row()
            .flex_1()
            .items_center()
            .gap(px(2.0))
            .min_w(px(0.0))
            .overflow_hidden();
        if let Some(message) = &self.message {
            left = left.child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.color(UiRole::Danger))
                    .child(message.clone()),
            );
        } else if let Some(notice) = &self.notice {
            left = left.child(
                div()
                    .debug_selector(|| "status-notice".into())
                    .flex_none()
                    .whitespace_nowrap()
                    .child(notice.clone()),
            );
        } else if let Some(project) = self.project(cx) {
            let (parts, icon) = self.status_path(&project, cx);
            let last = parts.len() - 1;
            for (i, part) in parts.into_iter().enumerate() {
                if i > 0 {
                    left = left.child(
                        div()
                            .flex_none()
                            .child(Icon::new(IconName::CHEVRON_RIGHT).size(px(16.0))),
                    );
                }
                let item = div()
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(4.0));
                let item = match (i == last, icon) {
                    (true, Some(icon)) => item.child(Icon::new(icon)),
                    _ => item,
                };
                left = left.child(item.whitespace_nowrap().child(part));
            }
        }
        let mut right = div().flex().flex_row().items_center().gap(px(2.0));
        // The Language Services widget comes right after the caret
        // position (first without an editor).
        let mut language_services = self
            .code_intel
            .as_ref()
            .and_then(|view| code_intel_ui::services::status_widget(view, cx));
        if let Some(editor) = self.active_editor() {
            let e = editor.read(cx);
            let p = e.caret_point();
            let (chars, breaks) = e.selection_stats();
            let mut caret = format!("{}:{}", p.line + 1, p.column + 1);
            if chars > 0 {
                caret.push_str(&format!(" ({chars} chars"));
                if breaks > 0 {
                    caret.push_str(&format!(", {breaks} line breaks"));
                }
                caret.push(')');
            }
            right = right
                .child(chrome::status_widget("status-caret", caret.into(), cx))
                .children(language_services.take())
                .child(chrome::status_widget(
                    "status-line-ending",
                    e.buffer().line_ending().label().into(),
                    cx,
                ))
                .child(chrome::status_widget(
                    "status-encoding",
                    e.buffer().encoding().label().into(),
                    cx,
                ))
                .child(chrome::status_widget(
                    "status-indent",
                    format!("{} spaces", e.config().indent).into(),
                    cx,
                ));
        }
        if let Some(widget) = language_services {
            right = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .child(widget)
                .child(right);
        }
        right = right.children(self.status_widgets.iter().cloned());
        if self
            .active_editor()
            .is_some_and(|e| e.read(cx).buffer().is_read_only())
        {
            right = right.child(Icon::new(IconName::LOCKED));
        }
        div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            // Too narrow, the widgets run past the left edge.
            .justify_end()
            .h(height)
            .px(px(10.0))
            .gap(px(16.0))
            .bg(theme.color(UiRole::StatusBar))
            .text_size(text_size)
            .text_color(theme.color(UiRole::StatusBarText))
            .child(left)
            .child(self.background_tasks.clone())
            .child(right.debug_selector(|| "status-right".into()).flex_none())
    }
}

impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bench = env::bench().is_some();
        if bench {
            crate::bench::frame_started();
        }
        let project_root = self.project_root(cx);
        self.background_tasks
            .update(cx, |tasks, cx| tasks.set_project(project_root, cx));
        let layout = self.frame_layout(window, cx);
        let run_widget = self.run_widget_state(window, cx);
        if let Some(tool_window) = self.focused_tool_window(window, cx) {
            self.tool_windows.set_last_active(tool_window);
        }
        // The frame around the islands has the toolbar colour.
        let frame = cx.theme().color(UiRole::Toolbar);
        let first_frame = (!self.first_frame_reported).then(|| {
            self.first_frame_reported = true;
            canvas(
                |_, _, _| (),
                |_, _, _, cx| {
                    let ms = env::since_main_ms();
                    cx.defer(move |cx| {
                        env::marker(&format!("first_frame main_ms={ms:.1}"));
                        if env::bench() == Some(&env::Bench::Startup) {
                            cx.quit();
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });
        let mut root = ui::components::root(cx)
            .id("workbench")
            .key_context("Workbench")
            .track_focus(&self.focus)
            .on_action(crate::simulate::simulate_mouse)
            .on_action(cx.listener(Self::toggle_project))
            .on_action(cx.listener(Self::toggle_run))
            .on_action(cx.listener(Self::activate_tool_window_action))
            .on_action(cx.listener(Self::toggle_tool_window_action))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::hide_tool_window))
            .on_action(cx.listener(Self::locate_file))
            .on_action(cx.listener(Self::set_theme))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::import_reference_settings))
            .on_action(cx.listener(Self::open_welcome_action))
            .on_action(cx.listener(Self::welcome_theme))
            .on_action(cx.listener(Self::welcome_keymap))
            .on_action(cx.listener(Self::new_script))
            .on_action(cx.listener(Self::show_gear_menu))
            .on_action(cx.listener(Self::show_project_widget))
            .on_action(cx.listener(Self::open_recent_project))
            .on_action(cx.listener(Self::copy_recent_path))
            .on_action(cx.listener(Self::remove_recent_project))
            .on_action(cx.listener(Self::reveal_recent_project))
            .on_action(cx.listener(Self::switch_theme))
            .on_action(cx.listener(Self::switch_keymap))
            .on_action(cx.listener(Self::quick_switch))
            .on_action(cx.listener(Self::open_folder_action))
            .on_action(cx.listener(Self::open_file_action))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_all_action))
            .on_action(cx.listener(Self::synchronize))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::select_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::show_go_to_line))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::open_find_in_files))
            .on_action(cx.listener(Self::open_replace_in_files))
            .on_action(cx.listener(Self::search_in_files))
            .on_action(cx.listener(Self::open_search_everywhere))
            .on_action(cx.listener(Self::go_to_file))
            .on_action(cx.listener(Self::go_to_action))
            .on_action(cx.listener(Self::go_to_text))
            .on_action(cx.listener(Self::open_recent_files))
            .on_action(cx.listener(Self::search_everywhere_for))
            .on_action(cx.listener(Self::run_run))
            .on_action(cx.listener(Self::run_current_file))
            .on_action(cx.listener(Self::run_stop))
            .on_action(cx.listener(Self::run_stop_all))
            .on_action(cx.listener(Self::run_rerun))
            .on_action(cx.listener(Self::run_close_tab))
            .on_action(cx.listener(Self::run_close_other_tabs))
            .on_action(cx.listener(Self::run_close_all_tabs))
            .on_action(cx.listener(Self::run_select_tab))
            .on_action(cx.listener(Self::run_toggle_pin))
            .on_action(cx.listener(Self::run_previous_link))
            .on_action(cx.listener(Self::run_next_link))
            .on_action(cx.listener(Self::run_scroll_to_end))
            .on_action(cx.listener(Self::run_clear))
            .on_action(cx.listener(Self::run_send_eof))
            .on_action(cx.listener(Self::run_type))
            .on_action(cx.listener(Self::run_choose_configuration))
            .on_action(cx.listener(Self::run_select_current_file))
            .on_action(cx.listener(Self::run_more_actions))
            .on_action(cx.listener(Self::run_select_configuration))
            .on_action(cx.listener(Self::run_launch_configuration))
            .on_action(cx.listener(Self::run_toggle_all_configurations))
            .on_action(cx.listener(Self::run_import_reference))
            .on_action(cx.listener(Self::run_file))
            .on_action(cx.listener(Self::run_edit_configurations))
            .on_action(cx.listener(Self::run_save_temporary))
            .on_action(cx.listener(Self::run_delete_configuration))
            .on_action(cx.listener(Self::run_pin_configuration))
            .on_action(cx.listener(Self::notify_action))
            .on_action(cx.listener(Self::clear_notifications))
            .map(|root| crate::background_tasks::on_actions(root, &self.background_tasks))
            // Keyboard input for the idle save; any key hides the find
            // bar's hint.
            .capture_key_down(cx.listener(|this, _, _, cx| {
                this.note_input(cx);
                if let Some(bar) = this.active_find_bar().cloned() {
                    bar.update(cx, |bar, cx| bar.clear_hint(cx));
                }
            }))
            // Mouse buttons of the keymap (Back and Forward).
            .capture_any_mouse_down(|e, window, cx| {
                if crate::keymap::mouse_down(e.button, window, cx) {
                    cx.stop_propagation();
                }
            })
            // Double taps of a modifier (Shift Shift).
            .on_modifiers_changed(|e: &gpui::ModifiersChangedEvent, window, cx| {
                crate::keymap::modifiers_changed(e.modifiers, window, cx)
            })
            .on_mouse_move(cx.listener(Self::resize_area))
            .when(env::debug_cursor(), |d| {
                d.on_mouse_move(|e: &MouseMoveEvent, window, _| {
                    env::marker(&format!(
                        "mouse {:.0},{:.0} active {} resolved {:?} shown {}",
                        f32::from(e.position.x),
                        f32::from(e.position.y),
                        window.is_window_active(),
                        ui::pointer::resolve(window),
                        crate::shot::current_cursor()
                    ))
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.end_resizing() {
                        cx.notify();
                    }
                }),
            );
        // Editor and tree commands reach the active editor and the tree
        // when another component has the focus.
        if let Some(editor) = self.active_editor() {
            root = editor::forward_actions(root, editor.clone());
        }
        if let Some(bar) = self.active_find_bar() {
            root = editor::find::forward_actions(root, bar.clone());
        }
        if let Some(view) = &self.code_intel {
            root = code_intel_ui::forward_actions(root, view.clone());
        }
        root = project_panel::forward_actions(root, self.panel.clone());
        root = self.vcs_actions(root, cx);
        root = self.branch_actions(root, cx);
        root = self.remote_actions(root, cx);
        root = self.debug_actions(root, cx);
        root = self.commit_actions(root, cx);
        root = self.terminal_actions(root, cx);
        root = self.diff_actions(root, cx);
        root = self.window_actions(root, cx);
        root = self.tab_menu_actions(root, cx);
        root = self.split_actions(root, cx);
        let marker_popup = self.vcs_render_popup(window, cx);
        let debug_overlay = self.debug_overlay(window, cx);
        root.flex()
            .flex_col()
            .bg(frame)
            .child(self.render_toolbar(window, &run_widget, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.render_left_stripe(window, cx))
                    .child(
                        // A tool window below the editor spans under the
                        // project tool window too, as in the reference
                        // IDE's default layout ("Widescreen tool window
                        // layout" off: `ToolWindowPane`). Areas below their
                        // minimums run past the pane's edges and are cut
                        // there.
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .pl(px(tool_window_areas::PANE_INSET))
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_1()
                                    .min_h(layout.min)
                                    .children(self.render_side_area(Area::Left, &layout, cx))
                                    .child(self.render_editor_area(layout.min, window, cx))
                                    .children(self.render_side_area(Area::Right, &layout, cx)),
                            )
                            .children(self.render_bottom_area(&layout, cx)),
                    )
                    .child(self.render_right_stripe(window, cx)),
            )
            .child(self.render_status_bar(cx))
            .child(self.notifications.clone())
            .child(self.processes_popup.clone())
            .children(self.render_go_to_line(cx))
            .children(self.render_run_menu(window))
            .children(self.render_tab_menu(window))
            .children(self.render_branch_overlays(window, cx))
            .children(self.render_project_widget_popup(window))
            .children(self.render_commit_overlays())
            .children(self.render_remote_overlays(window))
            .children(self.render_switch_popup(window))
            .children(ui::dialog_window::blocker(window, cx))
            .children(self.dialog.as_ref().map(|d| d.dialog.clone()))
            .children(self.panel.read(cx).overlays(window))
            .when(self.find_in_files.read(cx).is_open(), |d| {
                d.child(self.find_in_files.clone())
            })
            .when(self.search_everywhere.read(cx).is_open(), |d| {
                d.child(self.search_everywhere.clone())
            })
            .when(self.recent_files.read(cx).is_open(), |d| {
                d.child(self.recent_files.clone())
            })
            .children(self.render_find_hint(window.viewport_size().height, cx))
            .children(marker_popup)
            .children(debug_overlay)
            .children(
                self.code_intel
                    .as_ref()
                    .map(|view| view.read(cx).render_overlay(window, cx))
                    .unwrap_or_default(),
            )
            .children(first_frame)
            // Measurements: the end of the frame's paint.
            .when(bench, |d| {
                let editor = self.active_editor().cloned();
                let find = self.active_find_bar().cloned();
                let se = self.search_everywhere.clone();
                let completion = self.code_intel.as_ref().is_some_and(|v| {
                    v.read(cx)
                        .popups()
                        .is_shown(code_intel_ui::completion::CONTEXT)
                });
                d.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, cx| {
                            crate::bench::frame_painted(
                                editor.as_ref(),
                                find.as_ref(),
                                &se,
                                completion,
                                cx,
                            )
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            // While resizing, the resize pointer stays wherever the mouse is.
            .when_some(self.resizing_cursor(), |d, cursor| {
                d.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| ui::pointer::request_window(cursor, window),
                    )
                    .absolute()
                    .size_0(),
                )
            })
            // The pointer shape is set again on every mouse move (see
            // ui::pointer).
            .child(ui::pointer::frame())
            .child(
                canvas(|_, _, _| (), {
                    let this = cx.entity();
                    move |_, _, window, _| {
                        // Mouse input for the idle save.
                        let input = this.clone();
                        window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                input.update(cx, |w, cx| w.note_input(cx));
                            }
                        });
                        let input = this.clone();
                        window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                input.update(cx, |w, cx| w.note_input(cx));
                            }
                        });
                        let input = this.clone();
                        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                input.update(cx, |w, cx| w.note_input(cx));
                            }
                        });
                    }
                })
                .absolute()
                .size_0(),
            )
    }
}
