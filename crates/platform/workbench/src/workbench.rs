//! The main window's root view: toolbar, stripes, the project tool window,
//! editor tabs, the status bar, and the commands that work on them.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use editor::find::FindEvent;
use editor::{Editor, EditorEvent, FindBar};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, CursorStyle, DispatchPhase, Entity,
    FocusHandle, Focusable, FontWeight, HitboxBehavior, Hsla, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Render, ScrollWheelEvent,
    SharedString, Subscription, Window, canvas, div, prelude::*, px,
};
use project::Project;
use serde_json::json;
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button, island};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::app::Product;
use crate::chrome;
use crate::commands::*;
use crate::env;
use crate::file_types::FileTypes;
use crate::project_panel::{self, PanelEvent, ProjectPanel};
use crate::settings::SaveSettings;

/// Default width of the project tool window.
pub const PROJECT_WIDTH: f32 = 333.0;
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
/// Narrowest project tool window when resizing it.
const MIN_PROJECT_WIDTH: f32 = 120.0;
/// Narrowest editor area left when widening the project tool window.
const MIN_EDITOR_WIDTH: f32 = 200.0;
/// Room kept on each side of the file path in the middle of the toolbar
/// for the widgets at the toolbar's ends; a longer path is cut short.
const TOOLBAR_SIDE_ROOM: f32 = 320.0;
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

/// An open file.
struct Tab {
    path: PathBuf,
    editor: Entity<Editor>,
    /// The editor's find bar (shown while it is open).
    find: Entity<FindBar>,
    _subscriptions: [Subscription; 3],
}

/// The main window: toolbar, stripes, project tool window, editor area,
/// status bar.
pub struct Workbench {
    product: Rc<Product>,
    file_types: Rc<FileTypes>,
    focus: FocusHandle,
    project_visible: bool,
    project_width: Pixels,
    first_frame_reported: bool,
    panel: Entity<ProjectPanel>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    /// Background reads and writes in flight.
    pending: usize,
    /// Last error or notice, shown in the status bar.
    message: Option<SharedString>,
    /// The Go to Line field while it is open, and its blur subscription.
    go_to_line: Option<(Entity<Editor>, Subscription)>,
    /// Resizing the project tool window: mouse x and width at the start.
    resizing: Option<(Pixels, Pixels)>,
    save_settings: SaveSettings,
    /// The last keyboard or mouse input or edit (executor clock).
    last_input: Option<Instant>,
    /// A save after the idle time is scheduled.
    idle_save_pending: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub fn new(product: Rc<Product>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let file_types = Rc::new(FileTypes::new(&product.file_types));
        let panel = cx.new(|cx| ProjectPanel::new(file_types.clone(), cx));
        let subscriptions = vec![
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                PanelEvent::OpenFile(path) => this.open_file(path.clone(), window, cx),
            }),
            // Files are saved when the window loses the focus, as in the
            // reference IDE.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.save_settings.save_on_deactivation {
                    this.save_all(cx);
                }
            }),
        ];
        Self {
            product,
            file_types,
            focus,
            project_visible: true,
            project_width: px(PROJECT_WIDTH),
            first_frame_reported: false,
            panel,
            tabs: Vec::new(),
            active: None,
            pending: 0,
            message: None,
            go_to_line: None,
            resizing: None,
            save_settings: SaveSettings::default(),
            last_input: None,
            idle_save_pending: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn project_visible(&self) -> bool {
        self.project_visible
    }

    pub fn project_width(&self) -> Pixels {
        self.project_width
    }

    /// The Go to Line field, while it is open.
    pub fn go_to_line_field(&self) -> Option<&Entity<Editor>> {
        self.go_to_line.as_ref().map(|(field, _)| field)
    }

    /// The path shown in the middle of the toolbar: relative to the project,
    /// then the full path in brackets.
    pub fn title_path(&self, cx: &App) -> Option<String> {
        let path = self.active_editor()?.read(cx).path()?.clone();
        let full = path.display().to_string();
        let relative = self
            .project(cx)
            .and_then(|p| p.relative(&path))
            .map(|r| r.display().to_string())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        Some(format!("{relative} [{full}]"))
    }

    pub fn panel(&self) -> &Entity<ProjectPanel> {
        &self.panel
    }

    /// The editor of the active tab.
    pub fn active_editor(&self) -> Option<&Entity<Editor>> {
        self.active.map(|i| &self.tabs[i].editor)
    }

    /// The find bar of the active tab's editor.
    pub fn active_find_bar(&self) -> Option<&Entity<FindBar>> {
        self.active.map(|i| &self.tabs[i].find)
    }

    /// Paths of the open tabs, in order.
    pub fn tab_paths(&self) -> Vec<PathBuf> {
        self.tabs.iter().map(|t| t.path.clone()).collect()
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
            "tree": self.panel.read(cx).state_json(),
            "tool_windows": { "project": {
                "visible": self.project_visible,
                "width": f32::from(self.project_width),
            } },
            "title_path": self.title_path(cx),
            "go_to_line": self.go_to_line_field().map(|f| f.read(cx).text()),
            "tabs": self.tabs.iter().map(|t| json!({
                "path": rel(&t.path),
                "modified": t.editor.read(cx).is_modified(),
            })).collect::<Vec<_>>(),
            "active_tab": self.active,
            "editor": self.active_editor().map(|e| e.read(cx).state_json()),
            "find": self.active_find_bar().map(|f| f.read(cx).state_json(cx)),
            "busy": self.is_busy(cx),
            "message": self.message.as_ref().map(|m| m.to_string()),
            "theme": cx.theme().id(),
        })
    }

    // ---- project and files ----------------------------------------------------

    /// Opens a folder as the project.
    pub fn open_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if !path.is_dir() {
            self.message = Some(format!("{} is not a folder", path.display()).into());
            cx.notify();
            return;
        }
        self.save_all(cx);
        self.tabs.clear();
        self.active = None;
        self.project_visible = true;
        self.panel
            .update(cx, |panel, cx| panel.set_project(Project::new(path), cx));
        window.focus(&self.panel.focus_handle(cx));
        cx.notify();
    }

    fn resolve(&self, path: &str, cx: &App) -> PathBuf {
        let p = Path::new(path);
        match self.project(cx) {
            Some(project) if p.is_relative() => project.root().join(p),
            _ => p.to_owned(),
        }
    }

    /// Opens a file in a tab (or selects its tab); reading happens off the
    /// UI thread.
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.activate(i, window, cx);
            return;
        }
        self.pending += 1;
        // Reading, decoding and building the rope all happen off the UI
        // thread.
        let read = cx.background_spawn({
            let path = path.clone();
            async move {
                project::read_document(&path).map(|loaded| {
                    let read_only = loaded.read_only || loaded.decoded.lossy;
                    let mut buffer = text::Buffer::from_decoded(loaded.decoded);
                    buffer.set_read_only(read_only);
                    buffer
                })
            }
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

    fn add_tab(
        &mut self,
        path: PathBuf,
        buffer: text::Buffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.activate(i, window, cx);
            return;
        }
        let file_type = self.file_types.for_path(&path).cloned();
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(buffer, window, cx);
            editor.set_path(Some(path.clone()));
            if let Some(t) = &file_type {
                editor.set_config(t.editor_config());
                if let Some(factory) = &t.highlighter {
                    editor.set_highlighter(Some(factory()), cx);
                }
            }
            editor
        });
        let subscription = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                // Edits count as input for the idle save (they may also come
                // from commands rather than keys).
                this.note_input(cx);
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
        let index = self.tabs.len();
        self.tabs.push(Tab {
            path,
            editor,
            find,
            _subscriptions: [subscription, find_subscription, observe_find],
        });
        self.message = None;
        self.activate(index, window, cx);
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
        self.active = Some(index);
        window.focus(&self.tabs[index].editor.focus_handle(cx));
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

    /// Writes one tab's file if it is modified.
    fn save_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let editor = tab.editor.clone();
        let path = tab.path.clone();
        if !editor.read(cx).is_modified() {
            return;
        }
        let (bytes, point) = editor.update(cx, |e, _| {
            let point = e.buffer_mut().begin_save();
            (e.buffer().encoded(), point)
        });
        self.pending += 1;
        let write = cx.background_spawn({
            let path = path.clone();
            async move { project::write_document(&path, &bytes) }
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.pending -= 1;
                match result {
                    Ok(()) => editor.update(cx, |e, cx| {
                        e.buffer_mut().mark_saved_at(point);
                        cx.notify();
                    }),
                    Err(e) => {
                        this.message = Some(format!("Cannot save {}: {e}", path.display()).into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn save_all(&mut self, cx: &mut Context<Self>) {
        for i in 0..self.tabs.len() {
            self.save_tab(i, cx);
        }
    }

    /// Closes a tab; a modified file is saved first (no prompt).
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.save_tab(index, cx);
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.saturating_sub(1).min(self.tabs.len() - 1)),
            other => other,
        };
        match self.active {
            Some(a) => window.focus(&self.tabs[a].editor.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
        cx.notify();
    }

    // ---- commands -------------------------------------------------------------

    fn toggle_project(&mut self, _: &ToggleProject, window: &mut Window, cx: &mut Context<Self>) {
        let panel_focused = self.panel.focus_handle(cx).contains_focused(window, cx);
        if self.project_visible && panel_focused {
            self.project_visible = false;
            self.focus_editor(&FocusEditor, window, cx);
        } else {
            self.project_visible = true;
            window.focus(&self.panel.focus_handle(cx));
        }
        cx.notify();
    }

    fn hide_tool_window(
        &mut self,
        _: &HideToolWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_visible = false;
        self.focus_editor(&FocusEditor, window, cx);
        cx.notify();
    }

    fn locate_file(&mut self, _: &LocateFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .active_editor()
            .and_then(|e| e.read(cx).path().cloned())
        else {
            return;
        };
        self.project_visible = true;
        self.panel.update(cx, |panel, cx| panel.reveal(&path, cx));
        window.focus(&self.panel.focus_handle(cx));
        cx.notify();
    }

    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_editor() {
            Some(e) => window.focus(&e.focus_handle(cx)),
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
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |e, cx| e.go_to(line, column, cx));
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

    fn set_theme(&mut self, action: &SetTheme, _: &mut Window, cx: &mut Context<Self>) {
        match themes::Registry::with_builtin().resolve(&action.id) {
            Ok(theme) => ui::theme::set(cx, theme),
            Err(e) => {
                self.message = Some(e.to_string().into());
                cx.notify();
            }
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
        let path = self.resolve(&action.path, cx);
        self.open_file(path, window, cx);
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.active {
            self.save_tab(i, cx);
        }
    }

    fn save_all_action(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
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
        if let Some(a) = self.active {
            self.activate((a + 1) % self.tabs.len(), window, cx);
        }
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = self.active {
            self.activate((a + self.tabs.len() - 1) % self.tabs.len(), window, cx);
        }
    }

    // ---- drawing --------------------------------------------------------------

    fn render_toolbar(&self, window_width: Pixels, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let height = theme.metric(Metric::ToolbarHeight);
        let button = theme.metric(Metric::ToolbarButtonSize);
        let left = if cfg!(target_os = "macos") {
            px(chrome::TRAFFIC_LIGHTS_INSET)
        } else {
            px(8.0)
        };
        let project = self.project(cx);
        // The active file's path, centered in the toolbar right of the
        // window buttons.
        let path_widget = self.title_path(cx).map(|text| {
            let icon = self
                .active_editor()
                .and_then(|e| e.read(cx).path().map(|p| self.file_types.icon(p)))
                .unwrap_or(IconName::FILE_UNKNOWN);
            let room = window_width - left - px(2.0 * TOOLBAR_SIDE_ROOM);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(left)
                .right_0()
                .flex()
                .items_center()
                .justify_center()
                .when(room > px(80.0), |d| {
                    d.child(chrome::file_path_widget(icon, text, cx).max_w(room))
                })
        });
        div()
            .id("toolbar")
            .debug_selector(|| "toolbar".into())
            .relative()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(height)
            .pl(left)
            .pr(px(8.0))
            .gap(px(4.0))
            .bg(theme.color(UiRole::Toolbar))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                chrome::project_widget(project.as_ref().map(|p| p.name()), cx).on_click(
                    |_, window, cx| window.dispatch_action(Box::new(OpenFolder { path: None }), cx),
                ),
            )
            .child(div().flex_1())
            .child(chrome::run_widgets(cx))
            .child(div().w(px(8.0)))
            .child(
                icon_button("search", IconName::SEARCH, button, px(20.0), false, cx)
                    .tooltip(Tooltip::text("Search Everywhere", None)),
            )
            .child(
                icon_button("settings", IconName::SETTINGS, button, px(20.0), false, cx)
                    .tooltip(Tooltip::text("Settings", None)),
            )
            .children(path_widget)
    }

    fn render_left_stripe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shortcut: SharedString = if cfg!(target_os = "macos") {
            "⌘1".into()
        } else {
            "Alt+1".into()
        };
        let project = chrome::stripe_button(
            "stripe-project",
            IconName::TOOL_PROJECT,
            "Project",
            Some(shortcut),
            self.project_visible,
            cx,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            if this.project_visible {
                this.hide_tool_window(&HideToolWindow, window, cx);
            } else {
                this.toggle_project(&ToggleProject, window, cx);
            }
        }))
        .into_any_element();
        let top = vec![
            project,
            chrome::stripe_button(
                "stripe-commit",
                IconName::TOOL_COMMIT,
                "Commit",
                None,
                false,
                cx,
            )
            .into_any_element(),
            chrome::stripe_button(
                "stripe-structure",
                IconName::TOOL_STRUCTURE,
                "Structure",
                None,
                false,
                cx,
            )
            .into_any_element(),
        ];
        let mut bottom: Vec<_> = self
            .product
            .left_stripe_bottom
            .iter()
            .map(|item| {
                chrome::stripe_button(item.id, item.icon, item.title, None, false, cx)
                    .into_any_element()
            })
            .collect();
        for (id, icon, title) in [
            ("stripe-services", IconName::TOOL_SERVICES, "Services"),
            ("stripe-terminal", IconName::TOOL_TERMINAL, "Terminal"),
            ("stripe-problems", IconName::TOOL_PROBLEMS, "Problems"),
            ("stripe-vcs", IconName::TOOL_VCS, "Version Control"),
        ] {
            bottom.push(chrome::stripe_button(id, icon, title, None, false, cx).into_any_element());
        }
        chrome::stripe("left-stripe", top, bottom, cx)
    }

    fn render_right_stripe(&self, cx: &App) -> impl IntoElement {
        let top = vec![
            chrome::stripe_button(
                "stripe-notifications",
                IconName::TOOL_NOTIFICATIONS,
                "Notifications",
                None,
                false,
                cx,
            )
            .into_any_element(),
        ];
        chrome::stripe("right-stripe", top, Vec::new(), cx)
    }

    fn render_project(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
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
                            .child("Project"),
                    )
                    .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0))),
            )
            .child(div().flex_1())
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
            );
        island(cx, UiRole::ToolWindow)
            .id("project-tool-window")
            .debug_selector(|| "project-tool-window".into())
            .flex_none()
            .w(self.project_width)
            .h_full()
            .child(header)
            .child(self.panel.clone())
    }

    /// The gap between the project tool window and the editor; dragging it
    /// resizes the tool window.
    fn render_splitter(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("project-splitter")
            .debug_selector(|| "project-splitter".into())
            .flex_none()
            .w(width)
            .h_full()
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    |_, hitbox, window, _| {
                        ui::pointer::request(CursorStyle::ResizeLeftRight, &hitbox, window)
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.resizing = Some((e.position.x, this.project_width));
                    cx.stop_propagation();
                }),
            )
    }

    fn resize_project(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((x0, w0)) = self.resizing else {
            return;
        };
        if e.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            cx.notify();
            return;
        }
        let max = (window.viewport_size().width - px(MIN_EDITOR_WIDTH)).max(px(MIN_PROJECT_WIDTH));
        self.project_width = (w0 + e.position.x - x0).clamp(px(MIN_PROJECT_WIDTH), max);
        cx.notify();
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
            .border_color(theme.color(UiRole::Border))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow_lg()
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
                .border_color(theme.color(UiRole::Border))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow_md()
                .whitespace_nowrap()
                .child(hint),
        )
    }

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let selected = self.active == Some(index);
        let pill_h = theme.metric(Metric::TabSelectionHeight);
        let radius = theme.metric(Metric::ButtonRadius);
        let hover = theme.color(UiRole::TabHover);
        let active_bg = theme.color(UiRole::TabActive);
        let outline = theme.color(UiRole::TabUnderline);
        let text: Hsla = if selected {
            theme.color(UiRole::TabActiveText)
        } else {
            theme
                .color(UiRole::TabInactiveText)
                .opacity(INACTIVE_TAB_ALPHA)
        };
        let modified = tab.editor.read(cx).is_modified();
        let name = tab
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = name;
        let group: SharedString = format!("tab-{index}").into();
        // A modified file shows a dot where the close button is; hovering
        // the dot shows the close button (as the reference IDE's New UI tabs
        // do when modified tabs are marked).
        let close_group: SharedString = format!("tab-close-{index}").into();
        let dot = theme.color(UiRole::Accent);
        let close_icon = div()
            .absolute()
            .size_full()
            .when(modified, |d| {
                d.invisible()
                    .group_hover(close_group.clone(), |s| s.visible())
            })
            .child(Icon::new(IconName::CLOSE_SMALL));
        let marker = modified.then(|| {
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
            .when(!selected && !modified, |d| {
                d.invisible().group_hover(group.clone(), |s| s.visible())
            })
            .hover(move |s| s.bg(hover))
            .child(close_icon)
            .children(marker)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.close_tab(index, window, cx);
            }));
        let pill = div()
            .id(("tab", index))
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
            .when(selected, |d| d.bg(active_bg).border_color(outline))
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .hover(move |s| s.bg(hover))
            })
            .child(Icon::new(self.file_types.icon(&tab.path)))
            .child(div().whitespace_nowrap().text_color(text).child(title))
            .child(close)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate(index, window, cx);
            }));
        div()
            .flex_none()
            .px(px(TAB_H_OFFSET))
            .child(pill)
            .into_any_element()
    }

    fn render_editor_area(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(active) = self.active {
            let theme = cx.theme();
            let tab_h = theme.metric(Metric::TabHeight);
            let separator = theme.color(UiRole::Border);
            let tabs: Vec<AnyElement> = (0..self.tabs.len())
                .map(|i| self.render_tab(i, cx))
                .collect();
            return island(cx, UiRole::EditorBackground)
                .id("editor-area")
                .debug_selector(|| "editor-area".into())
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(
                    div()
                        .id("editor-tabs")
                        .debug_selector(|| "editor-tabs".into())
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .h(tab_h)
                        .pl(px(FIRST_TAB_OFFSET))
                        .overflow_x_scroll()
                        .border_b_1()
                        .border_color(separator)
                        .children(tabs),
                )
                .children(
                    self.tabs[active]
                        .find
                        .read(cx)
                        .is_open()
                        .then(|| self.tabs[active].find.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(self.tabs[active].editor.clone()),
                )
                .into_any_element();
        }
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let link = theme.color(UiRole::TextLink);
        let hint = |text: &'static str, keys: Option<&'static str>| {
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(div().text_color(muted).child(text))
                .children(keys.map(|k| div().text_color(link).child(k)))
        };
        let project_keys = if cfg!(target_os = "macos") {
            "⌘1"
        } else {
            "Alt+1"
        };
        island(cx, UiRole::EditorBackground)
            .id("editor-area")
            .debug_selector(|| "editor-area".into())
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(hint("Open a folder to start", None))
            .child(hint("Project tool window", Some(project_keys)))
            .child(hint("Drop files here to open them", None))
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (height, text_size) = chrome::status_metrics(cx);
        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .min_w(px(0.0));
        if let Some(message) = &self.message {
            left = left.child(
                div()
                    .whitespace_nowrap()
                    .text_color(theme.color(UiRole::Danger))
                    .child(message.clone()),
            );
        } else if let Some(project) = self.project(cx) {
            let mut parts: Vec<String> = vec![project.name().to_owned()];
            let mut icon = None;
            if let Some(editor) = self.active_editor()
                && let Some(path) = editor.read(cx).path()
                && let Some(rel) = project.relative(path)
            {
                parts.extend(rel.iter().map(|c| c.to_string_lossy().into_owned()));
                icon = Some(self.file_types.icon(path));
            }
            let last = parts.len() - 1;
            for (i, part) in parts.into_iter().enumerate() {
                if i > 0 {
                    left = left.child(Icon::new(IconName::CHEVRON_RIGHT).size(px(16.0)));
                }
                let item = div()
                    .flex()
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
        right = right.children(
            self.product
                .interpreter_label
                .map(|label| chrome::status_widget("status-interpreter", label.into(), cx)),
        );
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
            .justify_between()
            .h(height)
            .px(px(10.0))
            .gap(px(16.0))
            .bg(theme.color(UiRole::StatusBar))
            .text_size(text_size)
            .text_color(theme.color(UiRole::StatusBarText))
            .child(left.overflow_hidden())
            .child(right)
    }
}

impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::pointer::begin_frame();
        let bench = env::bench().is_some();
        if bench {
            crate::bench::frame_started();
        }
        let gap = cx.theme().metric(Metric::IslandGap);
        let window_width = window.viewport_size().width;
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
            .on_action(cx.listener(Self::toggle_project))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::hide_tool_window))
            .on_action(cx.listener(Self::locate_file))
            .on_action(cx.listener(Self::set_theme))
            .on_action(cx.listener(Self::open_folder_action))
            .on_action(cx.listener(Self::open_file_action))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_all_action))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::select_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::show_go_to_line))
            // Keyboard input for the idle save; any key hides the find
            // bar's hint.
            .capture_key_down(cx.listener(|this, _, _, cx| {
                this.note_input(cx);
                if let Some(bar) = this.active_find_bar().cloned() {
                    bar.update(cx, |bar, cx| bar.clear_hint(cx));
                }
            }))
            .on_mouse_move(cx.listener(Self::resize_project))
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
                    if this.resizing.take().is_some() {
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
        root = project_panel::forward_actions(root, self.panel.clone());
        root.flex()
            .flex_col()
            .bg(frame)
            .child(self.render_toolbar(window_width, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.render_left_stripe(cx))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .pl(px(0.5))
                            .when(self.project_visible, |d| {
                                d.child(self.render_project(cx))
                                    .child(self.render_splitter(gap, cx))
                            })
                            .child(self.render_editor_area(cx)),
                    )
                    .child(self.render_right_stripe(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(self.render_go_to_line(cx))
            .children(self.render_find_hint(window.viewport_size().height, cx))
            .children(first_frame)
            // Measurements: the end of the frame's paint.
            .when(bench, |d| {
                let editor = self.active_editor().cloned();
                let find = self.active_find_bar().cloned();
                d.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, cx| {
                            crate::bench::frame_painted(editor.as_ref(), find.as_ref(), cx)
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            // While resizing, the resize pointer stays wherever the mouse is.
            .when(self.resizing.is_some(), |d| {
                d.child(
                    canvas(
                        |_, _, _| (),
                        |_, _, window, _| {
                            ui::pointer::request_window(CursorStyle::ResizeLeftRight, window)
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            // The pointer shape is set again on every mouse move (see
            // ui::pointer).
            .child(
                canvas(|_, _, _| (), {
                    let this = cx.entity();
                    move |_, _, window, _| {
                        window.on_mouse_event(|_: &MouseMoveEvent, phase, window, _| {
                            if phase == DispatchPhase::Capture {
                                ui::pointer::apply(window);
                            }
                        });
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
