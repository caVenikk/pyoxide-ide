//! The Services tool window (bottom area): the consoles of the window as
//! a small tree, and for the selected one its Output log and result tabs
//! with the grid (the reference's Services tool window of a database
//! console, DB50, DB51).

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, Bounds, ClickEvent, Context, Corner, DismissEvent, Entity, EntityId,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*,
    px,
};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, header_tab, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, PopupOptions};
use ui::theme::ThemeExt;
use workbench::Workbench;
use workbench::tool_windows::{
    Area, ToolWindowContent, ToolWindowHook, ToolWindowSpec, ToolWindowView,
};
use workbench::workbench::ToolWindow;

use super::{Console, Pane, consoles};
use crate::icons;

/// The tool window's id: its stripe button is `stripe-services`.
pub const SERVICES: ToolWindow = ToolWindow("services");

const TREE_WIDTH: f32 = 230.0;
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;
const ROW_HEIGHT: f32 = 24.0;
const GROUP_KEY: &str = "";
const TOOLBAR_HEIGHT: f32 = 40.0;

/// The tool window's stripe button and how it is made.
pub(crate) fn hook() -> ToolWindowHook {
    ToolWindowHook {
        spec: ToolWindowSpec {
            id: SERVICES,
            area: Area::Bottom,
            order: 40,
            icon: icons::SERVICES,
            title: "Services",
            command: None,
            // Shown by the first console.
            registered: false,
        },
        create: std::rc::Rc::new(create),
        width: None,
    }
}

#[derive(Default)]
struct Panels(Vec<(AnyWindowHandle, WeakEntity<ServicesPanel>)>);

impl gpui::Global for Panels {}

fn create(
    workbench: WeakEntity<Workbench>,
    window: &mut Window,
    cx: &mut App,
) -> ToolWindowContent {
    let handle = window.window_handle();
    let panel = cx.new(|cx| ServicesPanel::new(handle, workbench, cx));
    let panels = cx.default_global::<Panels>();
    panels.0.retain(|(_, p)| p.upgrade().is_some());
    panels.0.push((handle, panel.downgrade()));
    ToolWindowContent::new(panel)
}

/// Shows `console` in its window's panel.
pub(crate) fn select(console: &Entity<Console>, window: AnyWindowHandle, cx: &mut App) {
    let panel = cx
        .try_global::<Panels>()
        .and_then(|p| p.0.iter().find(|(w, _)| *w == window))
        .and_then(|(_, p)| p.upgrade());
    if let Some(panel) = panel {
        let console = console.downgrade();
        panel.update(cx, |p, cx| {
            p.selected = Some(console);
            cx.notify();
        });
    }
}

fn panel_of(window: AnyWindowHandle, cx: &App) -> Option<Entity<ServicesPanel>> {
    cx.try_global::<Panels>()
        .and_then(|p| p.0.iter().find(|(w, _)| *w == window))
        .and_then(|(_, p)| p.upgrade())
}

/// Expands every row of the tree of `window`'s panel, or collapses them.
pub(crate) fn set_all_expanded(window: AnyWindowHandle, expanded: bool, cx: &mut App) {
    let Some(panel) = panel_of(window, cx) else {
        return;
    };
    let sources: Vec<String> = consoles(cx)
        .iter()
        .filter(|c| c.read(cx).window == window)
        .map(|c| c.read(cx).source.clone())
        .collect();
    panel.update(cx, |p, cx| {
        p.collapsed = if expanded {
            HashSet::new()
        } else {
            sources.into_iter().chain([GROUP_KEY.to_owned()]).collect()
        };
        cx.notify();
    });
}

/// The data sources and group rows of the panel that are collapsed.
pub(crate) fn collapsed(window: AnyWindowHandle, cx: &App) -> Vec<String> {
    panel_of(window, cx)
        .map(|p| {
            let mut rows: Vec<String> = p.read(cx).collapsed.iter().cloned().collect();
            rows.sort();
            rows
        })
        .unwrap_or_default()
}

pub(crate) fn init(_: &mut App) {}

struct ServicesPanel {
    window: AnyWindowHandle,
    #[allow(dead_code)]
    workbench: WeakEntity<Workbench>,
    focus: FocusHandle,
    selected: Option<WeakEntity<Console>>,
    observed: HashSet<EntityId>,
    subscriptions: Vec<Subscription>,
    /// The tree rows that are collapsed: the group (`""`) and data sources.
    collapsed: HashSet<String>,
    /// The open transaction popup, and where its button is drawn.
    tx_menu: Option<(Entity<ContextMenu>, Subscription)>,
    tx_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Focusable for ServicesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ToolWindowView for ServicesPanel {
    fn is_live(&self, cx: &App) -> bool {
        self.window_consoles(cx)
            .iter()
            .any(|c| c.read(cx).running.is_some())
    }
}

impl ServicesPanel {
    fn new(
        window: AnyWindowHandle,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            window,
            workbench,
            focus: cx.focus_handle(),
            selected: None,
            observed: HashSet::new(),
            subscriptions: Vec::new(),
            collapsed: HashSet::new(),
            tx_menu: None,
            tx_bounds: Rc::default(),
        }
    }

    /// Opens the transaction mode and isolation popup of `console`.
    fn open_tx(&mut self, console: &Entity<Console>, window: &mut Window, cx: &mut Context<Self>) {
        if self.tx_menu.is_some() {
            return;
        }
        let items = console.read(cx).tx_menu_items();
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.tx_menu = None;
            cx.notify();
        });
        self.tx_menu = Some((menu, subscription));
        cx.notify();
    }

    fn toggle(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_owned());
        }
        cx.notify();
    }

    fn window_consoles(&self, cx: &App) -> Vec<Entity<Console>> {
        consoles(cx)
            .into_iter()
            .filter(|c| c.read(cx).window == self.window)
            .collect()
    }

    /// The console shown: the selected one, else the newest.
    fn current(&self, cx: &App) -> Option<Entity<Console>> {
        self.selected
            .as_ref()
            .and_then(|c| c.upgrade())
            .filter(|c| consoles(cx).contains(c))
            .or_else(|| self.window_consoles(cx).into_iter().next_back())
    }

    fn render_header(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(theme.metric(Metric::ToolWindowHeaderHeight))
            .pl(px(12.0))
            .pr(px(8.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child("Services"),
            )
            .into_any_element()
    }

    /// The tree's toolbar: a new console, expand all, collapse all.
    fn render_tree_toolbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let divider = cx.theme().color(UiRole::Divider);
        let button = |id: &'static str, icon: IconName, tip: &'static str, cx: &mut App| {
            icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
                .debug_selector(move || id.to_owned())
                .tooltip(Tooltip::text(tip, None))
        };
        div()
            .id("services-tree-toolbar")
            .debug_selector(|| "services-tree-toolbar".to_owned())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(8.0))
            .gap(px(2.0))
            .child(
                button("services-new", IconName::ADD, "New Query Console", cx).on_click(
                    |_, window, cx| {
                        window.dispatch_action(Box::new(super::NewConsole { source: None }), cx)
                    },
                ),
            )
            .child(div().w(px(1.0)).h(px(16.0)).mx(px(3.0)).bg(divider))
            .child(
                button(
                    "services-expand-all",
                    IconName::EXPAND_ALL,
                    "Expand All",
                    cx,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(super::ExpandAll), cx)),
            )
            .child(
                button(
                    "services-collapse-all",
                    IconName::COLLAPSE_ALL,
                    "Collapse All",
                    cx,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(super::CollapseAll), cx)),
            )
            .into_any_element()
    }

    fn render_tree(
        &self,
        current: Option<&Entity<Console>>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let toolbar = self.render_tree_toolbar(cx);
        let theme = cx.theme();
        let consoles = self.window_consoles(cx);
        let muted = theme.color(UiRole::TextMuted);
        let hover = theme.color(UiRole::Hover);
        let selected_bg = theme.color(UiRole::Selection);
        let row = |id: String, depth: f32| {
            div()
                .id(gpui::SharedString::from(id))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(ROW_HEIGHT))
                .pl(px(4.0 + depth * 16.0))
                .pr(px(6.0))
                .rounded(theme.metric(Metric::ButtonRadius))
                .whitespace_nowrap()
        };
        // The expand arrow of a row, or room for it.
        let arrow = |key: Option<&str>, collapsed: bool, cx: &Context<Self>| {
            let slot = div().flex_none().w(px(ICON)).h(px(ICON));
            match key {
                Some(key) => {
                    let key = key.to_owned();
                    slot.debug_selector({
                        let key = key.clone();
                        move || format!("services-arrow-{key}")
                    })
                    .child(
                        Icon::new(if collapsed {
                            IconName::CHEVRON_RIGHT
                        } else {
                            IconName::CHEVRON_DOWN
                        })
                        .size(px(ICON)),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle(&key, cx);
                        }),
                    )
                }
                None => slot,
            }
        };
        let group_collapsed = self.collapsed.contains(GROUP_KEY);
        let mut tree = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .p(px(6.0))
            .pt(px(0.0))
            .gap(px(1.0))
            .child(
                row("services-group".into(), 0.0)
                    .child(arrow(Some(GROUP_KEY), group_collapsed, cx))
                    .child(Icon::new(icons::FOLDER).size(px(ICON)))
                    .child("Database"),
            );
        let mut seen: Vec<&str> = Vec::new();
        for console in &consoles {
            if group_collapsed {
                break;
            }
            let c = console.read(cx);
            let source_collapsed = self.collapsed.contains(&c.source);
            if !seen.contains(&c.source.as_str()) {
                seen.push(&c.source);
                tree = tree.child(
                    row(format!("services-source-{}", c.source), 1.0)
                        .child(arrow(Some(&c.source), source_collapsed, cx))
                        .child(Icon::new(icons::backend(c.backend)).size(px(ICON)))
                        .child(c.source.clone()),
                );
            }
            if source_collapsed {
                continue;
            }
            let is_current = current == Some(console);
            let console_entity = console.clone();
            let time = c.last_ms.map(|ms| format!("{ms} ms"));
            tree = tree.child(
                row(format!("services-console-{}", c.path.display()), 2.0)
                    .when(is_current, |d| d.bg(selected_bg))
                    .when(!is_current, |d| d.hover(move |s| s.bg(hover)))
                    .child(arrow(None, false, cx))
                    .child(Icon::new(icons::backend(c.backend)).size(px(ICON)))
                    .child(c.label())
                    .children(time.map(|t| div().text_color(muted).child(t)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.selected = Some(console_entity.downgrade());
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(TREE_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().color(UiRole::Border))
            .child(toolbar)
            .child(tree)
            .into_any_element()
    }

    fn render_tabs(
        &self,
        console: &Entity<Console>,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let c = console.read(cx);
        let running = c.running.is_some();
        let pane = c.pane;
        let mut tabs = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(40.0))
            .px(px(8.0))
            .gap(px(4.0));
        let output = console.clone();
        tabs = tabs.child(
            header_tab("services-tab-output", pane == Pane::Output, active, cx)
                .gap(px(4.0))
                .px(px(8.0))
                .child(Icon::new(icons::CONSOLE).size(px(ICON)))
                .child("Output")
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    output.update(cx, |c, cx| c.select_pane(Pane::Output, cx));
                })),
        );
        for (ix, tab) in c.tabs.iter().enumerate() {
            let select = console.clone();
            let close = console.clone();
            tabs = tabs.child(
                header_tab(
                    gpui::SharedString::from(format!("services-tab-{ix}")),
                    pane == Pane::Result(ix),
                    active,
                    cx,
                )
                .gap(px(4.0))
                .pl(px(8.0))
                .pr(px(2.0))
                .child(Icon::new(icons::TABLE).size(px(ICON)))
                .child(tab.title.clone())
                .child(
                    icon_button(
                        gpui::SharedString::from(format!("services-tab-close-{ix}")),
                        IconName::CLOSE_SMALL,
                        px(20.0),
                        px(14.0),
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        close.update(cx, |c, cx| c.close_tab(ix, cx));
                    })),
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    select.update(cx, |c, cx| c.select_pane(Pane::Result(ix), cx));
                })),
            );
        }
        let stop = console.clone();
        let tx_console = console.clone();
        // Transaction mode and Commit / Rollback (manual mode) come before
        // Stop; the reference has no divider between the tabs and them.
        tabs = tabs
            .child(
                crate::console::tx_ui::mode_button("services-tx", "Tx", self.tx_bounds.clone(), cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_tx(&tx_console, window, cx)
                    })),
            )
            .children(crate::console::tx_ui::commit_buttons(
                "services",
                console,
                px(BUTTON),
                px(ICON),
                cx,
            ));
        tabs.child(
            icon_button(
                "services-stop",
                if running {
                    IconName::STOP
                } else {
                    icons::STOP_DISABLED
                },
                px(BUTTON),
                px(ICON),
                false,
                cx,
            )
            .debug_selector(|| "services-stop".to_owned())
            .tooltip(Tooltip::text("Cancel Running Statements", None))
            .when(!running, |d| d.opacity(DISABLED_ALPHA))
            .when(running, |d| {
                d.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    stop.update(cx, |c, cx| c.stop(cx));
                }))
            }),
        )
        .into_any_element()
    }

    /// The output log's toolbar at its right edge: soft-wrap, scroll to
    /// the end, clear.
    fn render_output_toolbar(
        &self,
        console: &Entity<Console>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let c = console.read(cx);
        let wrapped = c.soft_wrap;
        let has_content = c.output.read(cx).has_content(cx);
        let selected = cx.theme().color(UiRole::Selection);
        let button = |id: &'static str, icon: IconName, tip: &'static str, cx: &mut App| {
            icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
                .debug_selector(move || id.to_owned())
                .tooltip(Tooltip::text(tip, None))
        };
        div()
            .id("services-output-toolbar")
            .debug_selector(|| "services-output-toolbar".to_owned())
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(32.0))
            .pt(px(2.0))
            .gap(px(2.0))
            .child(
                button(
                    "services-output-soft-wrap",
                    IconName::SOFT_WRAP,
                    "Soft-Wrap",
                    cx,
                )
                .when(wrapped, |d| d.bg(selected))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(super::ToggleOutputSoftWrap), cx)
                }),
            )
            .child(
                button(
                    "services-output-scroll-to-end",
                    IconName::SCROLL_DOWN,
                    "Scroll to End",
                    cx,
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(super::ScrollOutputToEnd), cx)
                }),
            )
            .child(
                button("services-output-clear", IconName::DELETE, "Clear All", cx)
                    .when(!has_content, |d| d.opacity(DISABLED_ALPHA))
                    .when(has_content, |d| {
                        d.on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(super::ClearOutput), cx)
                        })
                    }),
            )
            .into_any_element()
    }

    /// Keeps a subscription on every console, so that a change of one
    /// draws the panel.
    fn observe_consoles(&mut self, cx: &mut Context<Self>) {
        for console in self.window_consoles(cx) {
            if self.observed.insert(console.entity_id()) {
                self.subscriptions
                    .push(cx.observe(&console, |_, _, cx| cx.notify()));
            }
        }
    }
}

impl Render for ServicesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.observe_consoles(cx);
        let current = self.current(cx);
        let header = self.render_header(cx);
        let tree = self.render_tree(current.as_ref(), cx);
        let content = match &current {
            Some(console) => {
                let active = self.focus.contains_focused(window, cx);
                let tabs = self.render_tabs(console, active, cx);
                let c = console.read(cx);
                let body = match c.pane {
                    Pane::Output => div()
                        .flex()
                        .flex_row()
                        .size_full()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .child(c.output.clone()),
                        )
                        .child(self.render_output_toolbar(console, cx))
                        .into_any_element(),
                    Pane::Result(i) => match c.tabs.get(i) {
                        Some(tab) => tab.grid.clone().into_any_element(),
                        None => c.output.clone().into_any_element(),
                    },
                };
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(tabs)
                    .child(div().flex_1().min_h(px(0.0)).overflow_hidden().child(body))
            }
            None => div().flex_1(),
        };
        let tx_overlay = self.tx_menu.as_ref().map(|(menu, _)| {
            let anchor = self.tx_bounds.get();
            ui::menu::overlay_at(menu, anchor.bottom_left(), Corner::TopLeft, window)
        });
        div()
            .id("services-tool-window-content")
            .debug_selector(|| "services-tool-window-content".to_owned())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(tree)
                    .child(content),
            )
            .children(tx_overlay)
    }
}
