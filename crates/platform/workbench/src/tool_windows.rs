//! The tool window registry of the main window: which tool windows exist,
//! where they sit (left, bottom or right area) and which one each area
//! shows.
//!
//! The reference IDE's model (`ToolWindowManagerImpl`, `WindowInfoImpl`):
//! every tool window belongs to an area (its anchor) and has a place in the
//! stripe that holds its button; an area shows one tool window at a time
//! (no split mode yet), and the area keeps its size when another tool window
//! replaces the one it shows. In the new UI the left stripe holds the left
//! area's buttons at the top and the bottom area's buttons at the bottom;
//! the right stripe holds the right area's.

use std::rc::Rc;

use gpui::{AnyView, App, Entity, FocusHandle, Focusable, Pixels, Render, WeakEntity, Window, px};
use serde_json::json;
use ui::icons::IconName;

/// Default width of the project tool window (and of a side area).
pub const PROJECT_WIDTH: f32 = 333.0;
/// Height of the bottom area when it is first shown, as a share of the
/// space between the toolbar and the status bar (the reference IDE's
/// default tool window weight, `WindowInfoImpl.DEFAULT_WEIGHT`).
pub const BOTTOM_WEIGHT: f32 = 0.33;

/// A tool window's id (`ToolWindowId`): lower case, words joined with `-`.
/// The stripe button's debug selector is `stripe-{id}`, the tool window's
/// `{id}-tool-window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToolWindow(pub &'static str);

// Named like the variants of the enum this type replaced.
#[allow(non_upper_case_globals)]
impl ToolWindow {
    /// The project tool window (left area).
    pub const Project: ToolWindow = ToolWindow("project");
    /// The Run tool window (bottom area).
    pub const Run: ToolWindow = ToolWindow("run");
    /// The Debug tool window (bottom area).
    pub const Debug: ToolWindow = ToolWindow("debug");

    pub fn id(self) -> &'static str {
        self.0
    }
}

/// Where a tool window sits (`ToolWindowAnchor`; the top area is not
/// supported, as in the reference's new UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    Left,
    Bottom,
    Right,
}

impl Area {
    pub const ALL: [Area; 3] = [Area::Left, Area::Bottom, Area::Right];

    pub fn id(self) -> &'static str {
        match self {
            Area::Left => "left",
            Area::Bottom => "bottom",
            Area::Right => "right",
        }
    }

    /// The area is resized by its width (left, right) rather than by its
    /// height (bottom).
    pub fn is_side(self) -> bool {
        self != Area::Bottom
    }
}

/// The view a tool window shows. The workbench draws the area's frame (an
/// island); the view draws everything inside it, its header included.
pub trait ToolWindowView: Render + Focusable {
    /// Where activating the tool window moves the focus.
    fn content_focus(&self, cx: &App) -> FocusHandle {
        self.focus_handle(cx)
    }

    /// A process of the tool window is running: its stripe button shows
    /// the live indicator (`RunContentManagerImpl.setToolWindowIcon`).
    fn is_live(&self, _: &App) -> bool {
        false
    }
}

trait ErasedContent {
    fn view(&self) -> AnyView;
    fn focus(&self, cx: &App) -> FocusHandle;
    fn content_focus(&self, cx: &App) -> FocusHandle;
    fn is_live(&self, cx: &App) -> bool;
}

impl<V: ToolWindowView> ErasedContent for Entity<V> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn focus(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn content_focus(&self, cx: &App) -> FocusHandle {
        self.read(cx).content_focus(cx)
    }

    fn is_live(&self, cx: &App) -> bool {
        self.read(cx).is_live(cx)
    }
}

/// The content of a registered tool window.
#[derive(Clone)]
pub struct ToolWindowContent(Rc<dyn ErasedContent>);

impl ToolWindowContent {
    pub fn new<V: ToolWindowView>(view: Entity<V>) -> Self {
        Self(Rc::new(view))
    }

    pub fn view(&self) -> AnyView {
        self.0.view()
    }

    /// The tool window's own focus handle: the tool window is active while
    /// the focus is inside it.
    pub fn focus(&self, cx: &App) -> FocusHandle {
        self.0.focus(cx)
    }

    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        self.0.content_focus(cx)
    }

    pub fn is_live(&self, cx: &App) -> bool {
        self.0.is_live(cx)
    }
}

/// What the stripe shows of a tool window.
#[derive(Debug, Clone)]
pub struct ToolWindowSpec {
    pub id: ToolWindow,
    pub area: Area,
    /// Place among the area's stripe buttons (smaller first, top to
    /// bottom).
    pub order: u32,
    pub icon: IconName,
    pub title: &'static str,
    /// The command whose shortcut the stripe button's tooltip shows (the
    /// tool window's activation).
    pub command: Option<&'static str>,
    /// The tool window exists from the start and its button shows; Run and
    /// Debug instead are created by their first use.
    pub registered: bool,
}

/// Makes a module's tool window view for a main window.
pub type CreateToolWindow =
    Rc<dyn Fn(WeakEntity<crate::Workbench>, &mut Window, &mut App) -> ToolWindowContent>;

/// A tool window that a product's module contributes (ADR 0022): its
/// stripe button and how its view is made for each main window.
#[derive(Clone)]
pub struct ToolWindowHook {
    pub spec: ToolWindowSpec,
    pub create: CreateToolWindow,
}

impl std::fmt::Debug for ToolWindowHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolWindowHook")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

/// A tool window of the registry.
pub struct Entry {
    pub spec: ToolWindowSpec,
    /// `None` for a tool window the workbench draws itself (Project) and
    /// for stripe buttons that have no tool window yet.
    pub content: Option<ToolWindowContent>,
    registered: bool,
}

impl Entry {
    /// The stripe button shows.
    pub fn is_registered(&self) -> bool {
        self.registered
    }
}

/// One area: the tool window it shows and its size, which stays when
/// another tool window replaces that one.
#[derive(Debug, Clone, Copy)]
pub struct AreaSlot {
    pub visible: Option<ToolWindow>,
    /// The size the user gave the area, in pixels: the width of a side area,
    /// the height of the bottom area (`None` until the bottom area is first
    /// shown). It stays while the window is resized; the layout gives way
    /// below it only while the window is too small (`crate::layout`).
    pub size: Option<f32>,
}

/// The registry and the areas.
pub struct ToolWindows {
    entries: Vec<Entry>,
    left: AreaSlot,
    bottom: AreaSlot,
    right: AreaSlot,
    /// The tool window that had the focus last (Hide Active Tool Window).
    last_active: ToolWindow,
}

impl Default for ToolWindows {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolWindows {
    /// No tool windows; the left area shows Project once it is registered.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            left: AreaSlot {
                visible: Some(ToolWindow::Project),
                size: Some(PROJECT_WIDTH),
            },
            bottom: AreaSlot {
                visible: None,
                size: None,
            },
            right: AreaSlot {
                visible: None,
                size: Some(PROJECT_WIDTH),
            },
            last_active: ToolWindow::Project,
        }
    }

    /// Adds a tool window, or replaces the one with the same id (its
    /// visibility stays; a stripe button that had no tool window gets
    /// one).
    pub fn register(&mut self, spec: ToolWindowSpec, content: Option<ToolWindowContent>) {
        let id = spec.id;
        let old_area = self.area_of(id);
        match self.get_mut(id) {
            Some(entry) => {
                entry.registered |= spec.registered;
                entry.spec = spec;
                entry.content = content;
            }
            None => self.entries.push(Entry {
                registered: spec.registered,
                spec,
                content,
            }),
        }
        // A tool window moved to another area leaves the old one.
        if let Some(old) = old_area.filter(|a| Some(*a) != self.area_of(id)) {
            let slot = self.slot_mut(old);
            if slot.visible == Some(id) {
                slot.visible = None;
            }
        }
    }

    pub fn get(&self, id: ToolWindow) -> Option<&Entry> {
        self.entries.iter().find(|e| e.spec.id == id)
    }

    /// The tool window with this id.
    pub fn find(&self, id: &str) -> Option<ToolWindow> {
        self.entries
            .iter()
            .map(|e| e.spec.id)
            .find(|t| t.id() == id)
    }

    fn get_mut(&mut self, id: ToolWindow) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.spec.id == id)
    }

    /// Every tool window, by area (left, bottom, right) and order.
    pub fn all(&self) -> Vec<&Entry> {
        let mut all: Vec<&Entry> = self.entries.iter().collect();
        all.sort_by_key(|e| (area_rank(e.spec.area), e.spec.order));
        all
    }

    /// The stripe buttons of an area, in order.
    pub fn stripe(&self, area: Area) -> Vec<&Entry> {
        let mut buttons: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.spec.area == area && e.registered)
            .collect();
        buttons.sort_by_key(|e| e.spec.order);
        buttons
    }

    pub fn area_of(&self, id: ToolWindow) -> Option<Area> {
        self.get(id).map(|e| e.spec.area)
    }

    pub fn is_registered(&self, id: ToolWindow) -> bool {
        self.get(id).is_some_and(|e| e.registered)
    }

    /// Shows or takes away the tool window's stripe button; a tool window
    /// that leaves the stripe is hidden.
    pub fn set_registered(&mut self, id: ToolWindow, registered: bool) {
        if let Some(entry) = self.get_mut(id) {
            entry.registered = registered;
        }
        if !registered {
            self.hide(id);
        }
    }

    pub fn slot(&self, area: Area) -> &AreaSlot {
        match area {
            Area::Left => &self.left,
            Area::Bottom => &self.bottom,
            Area::Right => &self.right,
        }
    }

    fn slot_mut(&mut self, area: Area) -> &mut AreaSlot {
        match area {
            Area::Left => &mut self.left,
            Area::Bottom => &mut self.bottom,
            Area::Right => &mut self.right,
        }
    }

    /// The tool window an area shows, if it is still registered.
    pub fn visible_in(&self, area: Area) -> Option<ToolWindow> {
        self.slot(area)
            .visible
            .filter(|id| self.area_of(*id) == Some(area))
    }

    pub fn is_visible(&self, id: ToolWindow) -> bool {
        self.area_of(id)
            .is_some_and(|area| self.visible_in(area) == Some(id))
    }

    /// Shows a tool window in its area in place of the one there and
    /// registers it (its button shows from now on); `false` for an unknown
    /// id.
    pub fn show(&mut self, id: ToolWindow) -> bool {
        let Some(entry) = self.get_mut(id) else {
            return false;
        };
        entry.registered = true;
        let area = entry.spec.area;
        self.slot_mut(area).visible = Some(id);
        true
    }

    /// Hides the tool window if its area shows it.
    pub fn hide(&mut self, id: ToolWindow) {
        if let Some(area) = self.area_of(id) {
            let slot = self.slot_mut(area);
            if slot.visible == Some(id) {
                slot.visible = None;
            }
        }
    }

    /// Width the user gave a side area.
    pub fn width(&self, area: Area) -> Pixels {
        px(self.slot(area).size.unwrap_or(PROJECT_WIDTH))
    }

    /// The size the user gave an area; the bottom area takes
    /// [`BOTTOM_WEIGHT`] of `pane_height` until it has one.
    pub fn size(&self, area: Area, pane_height: f32) -> f32 {
        self.slot(area).size.unwrap_or(BOTTOM_WEIGHT * pane_height)
    }

    pub fn set_size(&mut self, area: Area, size: f32) {
        self.slot_mut(area).size = Some(size);
    }

    /// A shown bottom area keeps the height it was first shown with while
    /// the window is resized.
    pub fn keep_bottom_height(&mut self, pane_height: f32) {
        if self.visible_in(Area::Bottom).is_some() && self.bottom.size.is_none() {
            self.bottom.size = Some(BOTTOM_WEIGHT * pane_height);
        }
    }

    pub fn last_active(&self) -> ToolWindow {
        self.last_active
    }

    pub fn set_last_active(&mut self, id: ToolWindow) {
        self.last_active = id;
    }

    /// The tool windows by id, for `workbench.state`.
    pub fn state_json(&self) -> serde_json::Value {
        let mut windows = serde_json::Map::new();
        for entry in self.all() {
            let id = entry.spec.id;
            let area = entry.spec.area;
            let mut state = json!({
                "title": entry.spec.title,
                "area": area.id(),
                "registered": entry.registered,
                "visible": self.is_visible(id),
            });
            state[size_key(area)] = json!(self.slot(area).size);
            windows.insert(id.id().to_owned(), state);
        }
        windows.into()
    }

    /// The areas: the tool window each shows and its size.
    pub fn areas_json(&self) -> serde_json::Value {
        let mut areas = serde_json::Map::new();
        for area in Area::ALL {
            let mut state = json!({ "visible": self.visible_in(area).map(ToolWindow::id) });
            state[size_key(area)] = json!(self.slot(area).size);
            areas.insert(area.id().to_owned(), state);
        }
        areas.into()
    }
}

fn size_key(area: Area) -> &'static str {
    if area.is_side() { "width" } else { "height" }
}

fn area_rank(area: Area) -> u8 {
    match area {
        Area::Left => 0,
        Area::Bottom => 1,
        Area::Right => 2,
    }
}
