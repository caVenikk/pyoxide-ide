//! The tool window areas of the main window (left, bottom, right), their
//! stripe buttons, splitters and activation commands, on top of the
//! registry in `crate::tool_windows`.

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, FocusHandle, Focusable, HitboxBehavior,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, px,
};
use themes::UiRole;
use ui::components::island;
use ui::icons::IconName;
use ui::theme::ThemeExt;

use super::Workbench;
use crate::app::Product;
use crate::chrome;
use crate::commands::{
    ActivateToolWindow, FocusEditor, HideToolWindow, ToggleProject, ToggleRun, ToggleToolWindow,
};
use crate::notifications::{self, NotificationsToolWindow};
use crate::run::RunToolWindow;
use crate::tool_windows::{
    Area, MIN_BOTTOM_HEIGHT, MIN_EDITOR_HEIGHT, MIN_EDITOR_WIDTH, MIN_SIDE_WIDTH, ToolWindow,
    ToolWindowContent, ToolWindowSpec, ToolWindowView, ToolWindows,
};

/// Resizing an area by its splitter: the mouse position across the
/// splitter and the area's size at the start.
#[derive(Debug, Clone, Copy)]
pub(super) struct Resizing {
    area: Area,
    start: Pixels,
    size: f32,
}

impl ToolWindowView for RunToolWindow {
    fn content_focus(&self, cx: &App) -> FocusHandle {
        RunToolWindow::content_focus(self, cx)
    }

    fn is_live(&self, _: &App) -> bool {
        self.any_alive()
    }
}

/// The tool windows of the main window at start: the reference IDE's new
/// UI default stripes (`defaultToolWindowLayoutProvider.kt`,
/// `addPlatformDefaultsV2`), the product's bottom stripe buttons, Run,
/// which its first use registers, and Notifications. Buttons without a
/// tool window yet show on the stripe and do nothing. The orders leave
/// room for the tool windows to come (Debug after Run).
pub(super) fn default_tool_windows(
    product: &Product,
    run_window: &Entity<RunToolWindow>,
    notifications: &Entity<NotificationsToolWindow>,
) -> ToolWindows {
    let spec = |id, area, order, icon, title| ToolWindowSpec {
        id: ToolWindow(id),
        area,
        order,
        icon,
        title,
        command: None,
        registered: true,
    };
    let mut windows = ToolWindows::new();
    windows.register(
        ToolWindowSpec {
            command: Some("workbench.toggle-project"),
            ..spec("project", Area::Left, 0, IconName::TOOL_PROJECT, "Project")
        },
        None,
    );
    windows.register(
        spec("commit", Area::Left, 10, IconName::TOOL_COMMIT, "Commit"),
        None,
    );
    windows.register(
        spec(
            "structure",
            Area::Left,
            20,
            IconName::TOOL_STRUCTURE,
            "Structure",
        ),
        None,
    );
    windows.register(
        ToolWindowSpec {
            command: Some("workbench.toggle-run"),
            registered: false,
            ..spec("run", Area::Bottom, 0, IconName::TOOL_RUN, "Run")
        },
        Some(ToolWindowContent::new(run_window.clone())),
    );
    for (i, item) in product.left_stripe_bottom.iter().enumerate() {
        let id = item.id.strip_prefix("stripe-").unwrap_or(item.id);
        windows.register(
            spec(id, Area::Bottom, 20 + i as u32, item.icon, item.title),
            None,
        );
    }
    for (order, id, icon, title) in [
        (40, "services", IconName::TOOL_SERVICES, "Services"),
        (50, "terminal", IconName::TOOL_TERMINAL, "Terminal"),
        (60, "problems", IconName::TOOL_PROBLEMS, "Problems"),
        (70, "vcs", IconName::TOOL_VCS, "Version Control"),
    ] {
        windows.register(spec(id, Area::Bottom, order, icon, title), None);
    }
    windows.register(
        spec(
            notifications::ID,
            Area::Right,
            0,
            IconName::TOOL_NOTIFICATIONS,
            "Notifications",
        ),
        Some(ToolWindowContent::new(notifications.clone())),
    );
    windows
}

impl Workbench {
    /// The tool window registry and the areas.
    pub fn tool_windows(&self) -> &ToolWindows {
        &self.tool_windows
    }

    /// Adds a tool window or gives a stripe button its tool window.
    pub fn register_tool_window(
        &mut self,
        spec: ToolWindowSpec,
        content: Option<ToolWindowContent>,
        cx: &mut Context<Self>,
    ) {
        self.tool_windows.register(spec, content);
        cx.notify();
    }

    /// The handle that has the focus inside the tool window while it is
    /// active; `None` for a stripe button without a tool window.
    fn tool_window_focus(&self, id: ToolWindow, cx: &App) -> Option<FocusHandle> {
        if id == ToolWindow::Project {
            return Some(self.panel.focus_handle(cx));
        }
        Some(self.tool_windows.get(id)?.content.as_ref()?.focus(cx))
    }

    /// Where activating the tool window moves the focus.
    fn tool_window_content_focus(&self, id: ToolWindow, cx: &App) -> Option<FocusHandle> {
        if id == ToolWindow::Project {
            return Some(self.panel.focus_handle(cx));
        }
        Some(
            self.tool_windows
                .get(id)?
                .content
                .as_ref()?
                .content_focus(cx),
        )
    }

    /// The tool window that has the focus.
    pub fn focused_tool_window(&self, window: &Window, cx: &App) -> Option<ToolWindow> {
        self.tool_windows
            .all()
            .into_iter()
            .map(|e| e.spec.id)
            .find(|id| {
                self.tool_window_focus(*id, cx)
                    .is_some_and(|f| f.contains_focused(window, cx))
            })
    }

    /// The tool window is shown.
    pub fn is_visible(&self, id: ToolWindow) -> bool {
        self.tool_windows.is_visible(id)
    }

    /// Shows a tool window in its area, in place of the one there, and
    /// moves the focus into it; the first activation registers it
    /// (`ToolWindowEmptyStateAction`).
    pub fn activate_tool_window(
        &mut self,
        id: ToolWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(focus) = self.tool_window_content_focus(id, cx) else {
            return;
        };
        self.tool_windows.show(id);
        self.tool_windows.set_last_active(id);
        window.focus(&focus);
        cx.notify();
    }

    /// Hides a tool window; the focus goes to the editor.
    pub fn hide(&mut self, id: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.tool_windows.hide(id);
        self.focus_editor(&FocusEditor, window, cx);
        cx.notify();
    }

    /// The reference IDE's tool window activation: a tool window that has
    /// the focus is hidden, otherwise it is shown and focused.
    fn toggle_tool_window(&mut self, id: ToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .tool_window_focus(id, cx)
            .is_some_and(|f| f.contains_focused(window, cx));
        if self.is_visible(id) && focused {
            self.hide(id, window, cx);
        } else {
            self.activate_tool_window(id, window, cx);
        }
    }

    pub(super) fn toggle_project(
        &mut self,
        _: &ToggleProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_tool_window(ToolWindow::Project, window, cx);
    }

    pub(super) fn toggle_run(
        &mut self,
        _: &ToggleRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_tool_window(ToolWindow::Run, window, cx);
    }

    pub(super) fn activate_tool_window_action(
        &mut self,
        action: &ActivateToolWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.tool_windows.find(&action.id) {
            self.activate_tool_window(id, window, cx);
        }
    }

    pub(super) fn toggle_tool_window_action(
        &mut self,
        action: &ToggleToolWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.tool_windows.find(&action.id) {
            self.toggle_tool_window(id, window, cx);
        }
    }

    /// Hides the tool window that has the focus, or else the one that had
    /// it last (`HideToolWindowAction`).
    pub(super) fn hide_tool_window(
        &mut self,
        _: &HideToolWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self
            .focused_tool_window(window, cx)
            .or(Some(self.tool_windows.last_active()).filter(|t| self.is_visible(*t)))
            .unwrap_or(ToolWindow::Project);
        self.hide(id, window, cx);
    }

    /// The shortcut of a command, as the tooltips show it.
    fn command_shortcut(&self, command: &str, window: &Window, cx: &App) -> Option<SharedString> {
        let action = cx.build_action(command, None).ok()?;
        ui::keys::shortcut_in(action.as_ref(), &self.focus, window).map(Into::into)
    }

    /// The registered tool windows that can be shown, with their shortcuts
    /// (the tool windows of Recent Files).
    pub(super) fn tool_window_items(
        &self,
        window: &Window,
        cx: &App,
    ) -> Vec<crate::recent_files::ToolWindowItem> {
        self.tool_windows
            .all()
            .into_iter()
            .filter(|e| e.is_registered())
            .filter(|e| self.tool_window_focus(e.spec.id, cx).is_some())
            .map(|e| crate::recent_files::ToolWindowItem {
                id: e.spec.id.id(),
                title: e.spec.title,
                shortcut: e
                    .spec
                    .command
                    .and_then(|c| self.command_shortcut(c, window, cx)),
            })
            .collect()
    }

    /// The stripe buttons of an area's tool windows.
    fn stripe_buttons(
        &self,
        area: Area,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut buttons = Vec::new();
        for entry in self.tool_windows.stripe(area) {
            let id = entry.spec.id;
            let selector = SharedString::from(format!("stripe-{}", id.id()));
            let shortcut = entry
                .spec
                .command
                .and_then(|c| self.command_shortcut(c, window, cx));
            let selected = self.tool_windows.is_visible(id);
            let (icon, title) = (entry.spec.icon, entry.spec.title);
            let button = match &entry.content {
                // The live indicator while a process of the tool window
                // runs.
                Some(content) => {
                    let live = content
                        .is_live(cx)
                        .then(|| cx.theme().color(UiRole::Success));
                    chrome::stripe_button_badged(
                        selector.clone(),
                        icon,
                        title,
                        shortcut,
                        selected,
                        live,
                        cx,
                    )
                }
                None => {
                    chrome::stripe_button(selector.clone(), icon, title, shortcut, selected, cx)
                }
            };
            buttons.push(
                button
                    .debug_selector(move || selector.to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.is_visible(id) {
                            this.hide(id, window, cx);
                        } else {
                            this.activate_tool_window(id, window, cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }

    /// The left stripe: the left area's buttons at the top, the bottom
    /// area's at the bottom (the new UI).
    pub(super) fn render_left_stripe(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let top = self.stripe_buttons(Area::Left, window, cx);
        let bottom = self.stripe_buttons(Area::Bottom, window, cx);
        chrome::stripe("left-stripe", top, bottom, cx)
    }

    pub(super) fn render_right_stripe(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let top = self.stripe_buttons(Area::Right, window, cx);
        chrome::stripe("right-stripe", top, Vec::new(), cx)
    }

    /// The tool window an area shows, framed as an island.
    fn render_area(&self, area: Area, window: &Window, cx: &App) -> Option<AnyElement> {
        let id = self.tool_windows.visible_in(area)?;
        if id == ToolWindow::Project {
            return Some(self.render_project(cx).into_any_element());
        }
        let content = self.tool_windows.get(id)?.content.as_ref()?;
        let selector = format!("{}-tool-window", id.id());
        let frame = island(cx, UiRole::ToolWindow)
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .flex_none();
        let frame = match area {
            Area::Bottom => {
                let height =
                    (self.pane_height(window, cx) * self.tool_windows.bottom_weight()).max(px(0.0));
                frame.w_full().h(height)
            }
            _ => frame.w(self.tool_windows.width(area)).h_full(),
        };
        Some(frame.child(content.view()).into_any_element())
    }

    /// A side area and its splitter (on the editor's side), if the area
    /// shows a tool window.
    pub(super) fn render_side_area(
        &self,
        area: Area,
        gap: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(tool_window) = self.render_area(area, window, cx) else {
            return Vec::new();
        };
        let splitter = self.render_splitter(area, gap, cx).into_any_element();
        if area == Area::Left {
            vec![tool_window, splitter]
        } else {
            vec![splitter, tool_window]
        }
    }

    /// The bottom area below its splitter, if it shows a tool window.
    pub(super) fn render_bottom_area(
        &self,
        gap: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(tool_window) = self.render_area(Area::Bottom, window, cx) else {
            return Vec::new();
        };
        vec![
            self.render_splitter(Area::Bottom, gap, cx)
                .into_any_element(),
            tool_window,
        ]
    }

    /// The gap between an area and the editor; dragging it resizes the
    /// area.
    fn render_splitter(&self, area: Area, gap: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let id = match area {
            Area::Left => "project-splitter",
            Area::Bottom => "bottom-splitter",
            Area::Right => "right-splitter",
        };
        let cursor = resize_cursor(area);
        let splitter = div().id(id).debug_selector(move || id.into()).flex_none();
        let splitter = if area.is_side() {
            splitter.w(gap).h_full()
        } else {
            splitter.w_full().h(gap)
        };
        splitter
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |_, hitbox, window, _| ui::pointer::request(cursor, &hitbox, window),
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.resizing = Some(Resizing {
                        area,
                        start: if area.is_side() {
                            e.position.x
                        } else {
                            e.position.y
                        },
                        size: this.tool_windows.slot(area).size,
                    });
                    cx.stop_propagation();
                }),
            )
    }

    /// The pointer while an area is resized (it stays wherever the mouse
    /// is).
    pub(super) fn resizing_cursor(&self) -> Option<CursorStyle> {
        self.resizing.map(|r| resize_cursor(r.area))
    }

    /// Ends resizing; `true` when an area was being resized.
    pub(super) fn end_resizing(&mut self) -> bool {
        self.resizing.take().is_some()
    }

    /// Resizes the area whose splitter is being dragged.
    pub(super) fn resize_area(
        &mut self,
        e: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self.resizing else {
            return;
        };
        if e.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            cx.notify();
            return;
        }
        match r.area {
            Area::Bottom => {
                let pane = self.pane_height(window, cx);
                if pane <= px(0.0) {
                    return;
                }
                let max = (pane - px(MIN_EDITOR_HEIGHT)).max(px(MIN_BOTTOM_HEIGHT));
                let height =
                    (pane * r.size - (e.position.y - r.start)).clamp(px(MIN_BOTTOM_HEIGHT), max);
                self.tool_windows.set_size(Area::Bottom, height / pane);
            }
            side => {
                let other = if side == Area::Left {
                    Area::Right
                } else {
                    Area::Left
                };
                let taken = match self.tool_windows.visible_in(other) {
                    Some(_) => self.tool_windows.width(other),
                    None => px(0.0),
                };
                let max = (window.viewport_size().width - taken - px(MIN_EDITOR_WIDTH))
                    .max(px(MIN_SIDE_WIDTH));
                let delta = e.position.x - r.start;
                let delta = if side == Area::Left { delta } else { -delta };
                let width = (px(r.size) + delta).clamp(px(MIN_SIDE_WIDTH), max);
                self.tool_windows.set_size(side, f32::from(width));
            }
        }
        cx.notify();
    }
}

fn resize_cursor(area: Area) -> CursorStyle {
    if area.is_side() {
        CursorStyle::ResizeLeftRight
    } else {
        CursorStyle::ResizeUpDown
    }
}
