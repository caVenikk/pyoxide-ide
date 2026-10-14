//! The type engine widget of the status bar (`Types: ty`) and its "Type
//! Engine" popup, as the reference's `PyTypeEngineStatusBarWidget`, with
//! the `python.type-engine.*` commands.
//!
//! The reference shows the widget with a Pro subscription only and offers
//! its own engine and Pyrefly; Pyoxide shows it always and offers its type
//! servers, ty and basedpyright (ADR 0011). The popup lists them with a
//! check mark on the current one, a separator and "Type Engine
//! Settings…", which opens the project settings file until there is a
//! settings window.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, StatefulInteractiveElement,
    Styled, Subscription, WeakEntity, Window, canvas, div, prelude::FluentBuilder as _, px,
};
use py_lsp::services::{Problem, ProjectState};
use py_lsp::settings::{Tool, TypeEngine};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::command::CommandError;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use workbench::{StatusWidget, Workbench, WorkbenchEvent};

use crate::services::{AllowUvx, PythonServices, services_for};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the type engine popup.
    ChooseTypeEngine => "python.type-engine.choose", "Choose Type Engine",
        "Opens the Type Engine popup of the status bar: the type servers, a check mark on the current one, and Type Engine Settings.";
    /// Choose the project's type engine.
    SelectTypeEngine { name: String } => "python.type-engine.select", "Select Type Engine",
        "Makes ty or basedpyright the project's type server; the choice is kept in the project settings (`[python.type-engine]`) and the type server restarts.",
        r#"{"name": "basedpyright"}"#;
    /// Open where the type engine is set.
    TypeEngineSettings => "python.type-engine.settings", "Type Engine Settings\u{2026}",
        "Opens the project settings file (.oxide/project.toml), where the type engine and the Python tools are set, until there is a settings window.";
    /// Describe the widget.
    TypeEngineState => "python.type-engine.state", "Type Engine State",
        "Returns the type engine widget's state as JSON: whether it shows, its text and tooltip, the engine and the open popup.";
}

/// Title of the popup (`display.name` of the reference's type engine
/// bundle).
const POPUP_TITLE: &str = "Type Engine";
/// The last row of the popup (`open.settings`).
const SETTINGS_ROW: &str = "Type Engine Settings\u{2026}";

/// The widget's text (`Types: ty`).
pub fn text(engine: TypeEngine) -> String {
    format!("Types: {}", engine.name())
}

/// The widget's tooltip.
pub fn tooltip(engine: TypeEngine) -> String {
    format!("Selected type provider for python: {}", engine.name())
}

/// What needs the person: the tools waiting for the answer whether they
/// may run through uv, and why the tools that should run do not.
#[derive(Debug, Clone, Default, PartialEq)]
struct Attention {
    questions: Vec<Tool>,
    messages: Vec<String>,
}

impl Attention {
    fn of(state: &ProjectState) -> Self {
        let mut out = Self::default();
        for tool in state.tools.iter().filter(|t| t.runs) {
            let Some(problem) = tool.problem.as_ref().filter(|p| p.stops()) else {
                continue;
            };
            if *problem == Problem::NeedsUvxConsent {
                out.questions.push(tool.tool);
            }
            out.messages.push(problem.message(tool.tool));
        }
        out
    }

    fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// The row that asks whether `tool` may run through uv, with the answers
/// behind it.
fn uvx_row(tool: Tool) -> MenuItem {
    let answer = |allow| AllowUvx {
        tool: tool.id().to_owned(),
        allow,
    };
    MenuItem::submenu(
        format!("Run {} through uv (downloads it)", tool.name()),
        vec![
            MenuItem::action("Allow", answer(true)),
            MenuItem::action("Don't Allow", answer(false)),
        ],
    )
    .icon(IconName::INSPECTIONS_WARNING)
}

/// The popup's rows: the open uv questions and a separator, the engines
/// (the current one checked), a separator, the settings; and the current
/// engine's row.
fn popup_items(current: TypeEngine, questions: &[Tool]) -> (Vec<MenuItem>, Option<usize>) {
    let mut items: Vec<MenuItem> = questions.iter().map(|t| uvx_row(*t)).collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    let mut selected = None;
    for engine in TypeEngine::ALL {
        let mut row = MenuItem::action(
            engine.name(),
            SelectTypeEngine {
                name: engine.id().to_owned(),
            },
        );
        if engine == current {
            selected = Some(items.len());
            row = row.icon(IconName::CHECKED);
        }
        items.push(row);
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action(SETTINGS_ROW, TypeEngineSettings).icon(IconName::SETTINGS));
    (items, selected)
}

// ---- registration ----------------------------------------------------------------

/// The live widgets by main window.
#[derive(Default)]
struct Widgets(Vec<(AnyWindowHandle, WeakEntity<TypeEngineWidget>)>);

impl Global for Widgets {}

fn widget_of(window: AnyWindowHandle, cx: &App) -> Option<Entity<TypeEngineWidget>> {
    cx.try_global::<Widgets>()?
        .0
        .iter()
        .filter(|(w, _)| *w == window)
        .find_map(|(_, v)| v.upgrade())
}

/// The widget a command goes to: the active main window's, else the
/// newest.
fn target(cx: &App) -> Option<(AnyWindowHandle, Entity<TypeEngineWidget>)> {
    let widgets = cx.try_global::<Widgets>()?;
    let live: Vec<_> = widgets
        .0
        .iter()
        .filter_map(|(w, v)| v.upgrade().map(|v| (*w, v)))
        .collect();
    let active = cx.active_window();
    live.iter()
        .find(|(w, _)| Some(*w) == active)
        .cloned()
        .or_else(|| live.last().cloned())
}

/// Registers the widget's commands (the product's `init`).
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ChooseTypeEngine, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.open(window, cx));
            });
        });
    });
    cx.on_action(|action: &SelectTypeEngine, cx| {
        let Some(engine) = TypeEngine::parse(&action.name) else {
            return;
        };
        if let Some((_, widget)) = target(cx) {
            widget.update(cx, |w, cx| w.select(engine, cx));
        }
    });
    cx.on_action(|_: &TypeEngineSettings, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.open_settings(window, cx));
            });
        });
    });
    ui::command::register_query(
        cx,
        "python.type-engine.state",
        Rc::new(|_, window, cx| {
            let widget = widget_of(window.window_handle(), cx)
                .ok_or_else(|| CommandError::new("not_available", "no type engine widget"))?;
            Ok(widget.read(cx).state_json(cx))
        }),
    );
}

/// The type engine widget of the status bar.
pub fn status_widget() -> StatusWidget {
    StatusWidget {
        id: "status-type-engine",
        create: Rc::new(|workbench, window, cx| {
            let view = cx.new(|cx| TypeEngineWidget::new(workbench, cx));
            let handle = window.window_handle();
            let widgets = cx.default_global::<Widgets>();
            widgets.0.retain(|(_, v)| v.upgrade().is_some());
            widgets.0.push((handle, view.downgrade()));
            view.into()
        }),
    }
}

// ---- the widget ------------------------------------------------------------------

/// The type engine widget of one main window.
pub(crate) struct TypeEngineWidget {
    workbench: WeakEntity<Workbench>,
    root: Option<PathBuf>,
    /// The file of the active editor: the widget shows while it is a
    /// Python file of the project.
    active_file: Option<PathBuf>,
    services: Option<(Entity<PythonServices>, Subscription)>,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Where the widget was drawn (window coordinates), for the popup.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl TypeEngineWidget {
    fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let mut root = None;
        let mut active_file = None;
        if let Some(workbench) = workbench.upgrade() {
            root = workbench.read(cx).project_root(cx);
            active_file = workbench.read(cx).active_file(cx);
            subscriptions.push(
                cx.subscribe(
                    &workbench,
                    |this, _, event: &WorkbenchEvent, cx| match event {
                        WorkbenchEvent::ProjectOpened(root) => {
                            this.set_root(Some(root.clone()), cx)
                        }
                        WorkbenchEvent::ActiveFileChanged(file) => {
                            this.active_file = file.clone();
                            cx.notify();
                        }
                        _ => {}
                    },
                ),
            );
        }
        let mut this = Self {
            workbench,
            root: None,
            active_file,
            services: None,
            menu: None,
            bounds: Rc::default(),
            _subscriptions: subscriptions,
        };
        this.set_root(root, cx);
        this
    }

    /// Another project: follow its services.
    fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.services = root.as_ref().and_then(|root| {
            let services = services_for(root, cx).into_iter().next()?;
            let subscription = cx.observe(&services, |_, _, cx| cx.notify());
            Some((services, subscription))
        });
        self.root = root;
        self.menu = None;
        cx.notify();
    }

    /// The type engine in effect, once the services know it.
    fn engine(&self, cx: &App) -> Option<TypeEngine> {
        self.services.as_ref()?.0.read(cx).type_engine()
    }

    /// The active editor shows a Python file of the project (`.py`,
    /// `.pyi`), as the reference shows the widget.
    fn in_python_file(&self) -> bool {
        match (&self.root, &self.active_file) {
            (Some(root), Some(file)) => {
                file.starts_with(root) && file.extension().is_some_and(|e| e == "py" || e == "pyi")
            }
            _ => false,
        }
    }

    /// What needs the person, from the services' last preparation.
    fn attention(&self, cx: &App) -> Attention {
        self.services
            .as_ref()
            .and_then(|(s, _)| s.read(cx).state().map(Attention::of))
            .unwrap_or_default()
    }

    /// The widget's text, tooltip and warning mark; `None` hides it. A
    /// tool that should run and does not (or waits for an answer) marks
    /// the widget and says why in the tooltip.
    fn presentation(&self, cx: &App) -> Option<(String, String, bool)> {
        if !self.in_python_file() {
            return None;
        }
        let engine = self.engine(cx)?;
        let attention = self.attention(cx);
        let mut tooltip = tooltip(engine);
        for message in &attention.messages {
            tooltip.push('\n');
            tooltip.push_str(message);
        }
        Some((text(engine), tooltip, !attention.is_empty()))
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() || self.presentation(cx).is_none() {
            return;
        }
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let (items, selected) = popup_items(engine, &self.attention(cx).questions);
        let options = PopupOptions {
            title: Some(POPUP_TITLE.into()),
            speed_search: false,
            selected,
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    fn select(&mut self, engine: TypeEngine, cx: &mut Context<Self>) {
        if let Some((services, _)) = &self.services {
            services.update(cx, |s, cx| s.select_type_engine(engine, cx));
        }
    }

    /// Opens `.oxide/project.toml`, created (with an empty
    /// `[python.tools]` table) when the project has none yet.
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(root), Some(workbench)) = (self.root.clone(), self.workbench.upgrade()) else {
            return;
        };
        let file = project::store::settings_file(&root);
        let create = cx.background_spawn({
            let root = root.clone();
            async move { py_lsp::settings::ensure_tools_table(&root) }
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Err(e) = create.await {
                eprintln!("type engine settings: {e}");
            }
            let _ = workbench.update_in(cx, |w, window, cx| w.open_file(file, window, cx));
        })
        .detach();
    }

    fn state_json(&self, cx: &App) -> Json {
        let presentation = self.presentation(cx);
        json!({
            "visible": presentation.is_some(),
            "text": presentation.as_ref().map(|p| p.0.clone()),
            "tooltip": presentation.as_ref().map(|p| p.1.clone()),
            "warning": presentation.as_ref().is_some_and(|p| p.2),
            "uvx-questions": self
                .attention(cx)
                .questions
                .iter()
                .map(|t| t.id())
                .collect::<Vec<_>>(),
            "engine": self.engine(cx).map(TypeEngine::id),
            "engines": TypeEngine::ALL.map(TypeEngine::id),
            "popup": self.menu.as_ref().map(|(m, _)| m.read(cx).state_json()),
        })
    }
}

impl Render for TypeEngineWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((text, tooltip, warning)) = self.presentation(cx) else {
            return div().into_any_element();
        };
        let hover = cx.theme().color(UiRole::Hover);
        let bounds = self.bounds.clone();
        let overlay = self.menu.as_ref().map(|(menu, _)| {
            ui::menu::overlay_at(menu, self.bounds.get().origin, Corner::BottomLeft, window)
        });
        div()
            .id("status-type-engine")
            .debug_selector(|| "status-type-engine".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .whitespace_nowrap()
            .hover(move |s| s.bg(hover))
            .when(warning, |d| {
                d.child(Icon::new(IconName::INSPECTIONS_WARNING))
            })
            .child(text)
            .tooltip(Tooltip::text(tooltip, None))
            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx)))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(overlay)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_and_rows() {
        assert_eq!(text(TypeEngine::Ty), "Types: ty");
        assert_eq!(
            tooltip(TypeEngine::Basedpyright),
            "Selected type provider for python: Basedpyright"
        );
        let (items, selected) = popup_items(TypeEngine::Basedpyright, &[]);
        let labels: Vec<_> = items
            .iter()
            .map(|i| i.label().map(|l| l.to_string()))
            .collect();
        assert_eq!(
            labels,
            [
                Some("ty".into()),
                Some("Basedpyright".into()),
                None,
                Some(SETTINGS_ROW.into())
            ]
        );
        assert_eq!(selected, Some(1));

        let (items, selected) = popup_items(TypeEngine::Ty, &[Tool::Ruff, Tool::Ty]);
        let label = |i: usize| items[i].label().map(|l| l.to_string());
        assert_eq!(
            label(0).as_deref(),
            Some("Run Ruff through uv (downloads it)")
        );
        assert_eq!(
            label(1).as_deref(),
            Some("Run ty through uv (downloads it)")
        );
        assert_eq!(label(2), None);
        assert_eq!(label(3).as_deref(), Some("ty"));
        assert_eq!(selected, Some(3));
    }
}
