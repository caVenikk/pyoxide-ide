//! The tool window areas of the main window (left, bottom, right), their
//! stripe buttons, splitters and activation commands, on top of the
//! registry in `crate::tool_windows`.

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, FocusHandle, Focusable, HitboxBehavior,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, SharedString, Size, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, px, size,
};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::components::island;
use ui::icons::IconName;
use ui::theme::ThemeExt;

use super::Workbench;
use crate::app::Product;
use crate::chrome;
use crate::commands::{
    ActivateToolWindow, FocusEditor, HideToolWindow, ToggleProject, ToggleRun, ToggleToolWindow,
};
use crate::layout;
use crate::notifications::{self, NotificationsToolWindow};
use crate::run::RunToolWindow;
use crate::tool_windows::{
    Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView, ToolWindows,
};

/// Space between the left stripe and the column of the areas.
pub(super) const PANE_INSET: f32 = 0.5;

/// Resizing an area by its splitter: the mouse position across the
/// splitter and the area's size on screen at the start.
#[derive(Debug, Clone, Copy)]
pub(super) struct Resizing {
    area: Area,
    start: Pixels,
    size: f32,
}

/// Sizes of the main window's areas for the window's current size
/// (`crate::layout`).
#[derive(Debug, Clone, Copy)]
pub struct MainLayout {
    /// The space the areas share: between the stripes, below the toolbar
    /// and above the status bar.
    pub pane: Size<Pixels>,
    /// Between two areas (their splitter).
    pub gap: Pixels,
    /// The smallest island of an area.
    pub min: Pixels,
    pub left: Option<Pixels>,
    pub editor: Pixels,
    pub right: Option<Pixels>,
    /// The row of the editor and the side areas, above the bottom area.
    pub row: Pixels,
    pub bottom: Option<Pixels>,
}

impl MainLayout {
    /// The size of a shown area on screen: the width of a side area, the
    /// height of the bottom area.
    pub fn area(&self, area: Area) -> Option<Pixels> {
        match area {
            Area::Left => self.left,
            Area::Bottom => self.bottom,
            Area::Right => self.right,
        }
    }

    /// The sizes for `workbench.layout`.
    pub fn to_json(&self) -> serde_json::Value {
        let v = |p: Pixels| f32::from(p);
        let o = |p: Option<Pixels>| p.map(f32::from);
        json!({
            "pane": {"width": v(self.pane.width), "height": v(self.pane.height)},
            "gap": v(self.gap),
            "min": v(self.min),
            "left": o(self.left),
            "editor": v(self.editor),
            "right": o(self.right),
            "row": v(self.row),
            "bottom": o(self.bottom),
        })
    }
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

    /// The area shows a tool window that can be drawn.
    fn area_shown(&self, area: Area) -> bool {
        self.tool_windows.visible_in(area).is_some_and(|id| {
            id == ToolWindow::Project
                || self
                    .tool_windows
                    .get(id)
                    .is_some_and(|e| e.content.is_some())
        })
    }

    /// Sizes of the areas for the window's current size: the editor gives
    /// way first, then the tool window areas, down to their minimum
    /// (`crate::layout`).
    pub fn main_layout(&self, window: &Window, cx: &App) -> MainLayout {
        let theme = cx.theme();
        let gap = f32::from(theme.metric(Metric::IslandGap));
        let min = layout::min_island(gap);
        let width =
            f32::from(window.viewport_size().width - theme.metric(Metric::StripeWidth) * 2.0)
                - PANE_INSET;
        let height = f32::from(self.pane_height(window, cx));
        let shown = |area| {
            self.area_shown(area)
                .then(|| self.tool_windows.size(area, height))
        };
        let across = layout::split(width, shown(Area::Left), shown(Area::Right), gap, min);
        let down = layout::split(height, None, shown(Area::Bottom), gap, min);
        let if_shown = |extent: f32, area| self.area_shown(area).then(|| px(extent));
        MainLayout {
            pane: size(px(width), px(height)),
            gap: px(gap),
            min: px(min),
            left: if_shown(across.first, Area::Left),
            editor: px(across.inner),
            right: if_shown(across.last, Area::Right),
            row: px(down.inner),
            bottom: if_shown(down.last, Area::Bottom),
        }
    }

    /// The layout of the frame being drawn; a bottom area shown for the
    /// first time gets its height.
    pub(super) fn frame_layout(&mut self, window: &Window, cx: &App) -> MainLayout {
        let height = f32::from(self.pane_height(window, cx));
        self.tool_windows.keep_bottom_height(height);
        self.main_layout(window, cx)
    }

    /// The tool window an area shows, framed as an island at its size in
    /// `layout`.
    fn render_area(&self, area: Area, layout: &MainLayout, cx: &App) -> Option<AnyElement> {
        let id = self.tool_windows.visible_in(area)?;
        let size = layout.area(area)?;
        if id == ToolWindow::Project {
            return Some(self.render_project(size, cx).into_any_element());
        }
        let content = self.tool_windows.get(id)?.content.as_ref()?;
        let selector = format!("{}-tool-window", id.id());
        let frame = island(cx, UiRole::ToolWindow)
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .flex_none();
        let frame = match area {
            Area::Bottom => frame.w_full().h(size),
            _ => frame.w(size).h_full(),
        };
        Some(frame.child(content.view()).into_any_element())
    }

    /// A side area and its splitter (on the editor's side), if the area
    /// shows a tool window.
    pub(super) fn render_side_area(
        &self,
        area: Area,
        layout: &MainLayout,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(tool_window) = self.render_area(area, layout, cx) else {
            return Vec::new();
        };
        let splitter = self
            .render_splitter(area, layout.gap, cx)
            .into_any_element();
        if area == Area::Left {
            vec![tool_window, splitter]
        } else {
            vec![splitter, tool_window]
        }
    }

    /// The bottom area below its splitter, if it shows a tool window.
    pub(super) fn render_bottom_area(
        &self,
        layout: &MainLayout,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(tool_window) = self.render_area(Area::Bottom, layout, cx) else {
            return Vec::new();
        };
        vec![
            self.render_splitter(Area::Bottom, layout.gap, cx)
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
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    // The drag starts from the size on screen, which is
                    // smaller than the one the user gave while the window
                    // is too small.
                    let Some(size) = this.main_layout(window, cx).area(area) else {
                        return;
                    };
                    this.resizing = Some(Resizing {
                        area,
                        start: if area.is_side() {
                            e.position.x
                        } else {
                            e.position.y
                        },
                        size: f32::from(size),
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
        // The reference's divider: the area gets the size the mouse gives
        // it, at least the minimum, and at most what leaves the editor its
        // minimum next to the other side's size.
        let main = self.main_layout(window, cx);
        let (gap, min) = (f32::from(main.gap), f32::from(main.min));
        match r.area {
            Area::Bottom => {
                let wanted = r.size - f32::from(e.position.y - r.start);
                let height = layout::drag(wanted, f32::from(main.pane.height), None, gap, min);
                self.tool_windows.set_size(Area::Bottom, height);
            }
            side => {
                let other = if side == Area::Left {
                    Area::Right
                } else {
                    Area::Left
                };
                let other = self
                    .area_shown(other)
                    .then(|| f32::from(self.tool_windows.width(other)));
                let delta = f32::from(e.position.x - r.start);
                let delta = if side == Area::Left { delta } else { -delta };
                let width =
                    layout::drag(r.size + delta, f32::from(main.pane.width), other, gap, min);
                self.tool_windows.set_size(side, width);
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
