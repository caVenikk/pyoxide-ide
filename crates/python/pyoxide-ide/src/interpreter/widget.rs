//! The interpreter widget of the status bar and its "Python Interpreter"
//! popup, with the `python.interpreter.*` commands.
//!
//! The status bar is never on the focus path, so the commands are handled
//! application-wide and reach the widget of the active main window (or the
//! only one).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, StatefulInteractiveElement,
    Styled, Subscription, Task, WeakEntity, Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::command::CommandError;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName, IconSvg};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use workbench::{StatusWidget, Workbench, WorkbenchEvent};

use super::{FoundInterpreter, Group, ProjectInterpreters};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the interpreter popup.
    ChooseInterpreter => "python.interpreter.choose", "Switch Python Interpreter",
        "Opens the Python Interpreter popup of the status bar: the project's environments, the current one selected.";
    /// Choose the project's interpreter.
    SelectInterpreter { path: String } => "python.interpreter.select", "Select Python Interpreter",
        "Makes an interpreter (or the environment folder holding it) the project's interpreter and keeps the choice in the personal project state; a relative path is resolved against the project.",
        r#"{"path": ".venv"}"#;
    /// Describe the widget.
    InterpreterState => "python.interpreter.state", "Python Interpreter State",
        "Returns the interpreter widget's state as JSON: its text and tooltip, the selected interpreter, the project's environments and the open popup.";
}

/// Title of the popup (`configurable.PyActiveSdkModuleConfigurable.python.
/// interpreter.display.name`).
const POPUP_TITLE: &str = "Python Interpreter";
/// The widget's text without an interpreter (`noInterpreterMarker`).
const NO_INTERPRETER: &str = "<No interpreter>";
/// Rows of the popup that come with the interpreter settings (#75).
const ADD_NEW_INTERPRETER: &str = "Add New Interpreter";
const INTERPRETER_SETTINGS: &str = "Interpreter Settings\u{2026}";
const MANAGE_PACKAGES: &str = "Manage Packages\u{2026}";

/// Icons: the flavour icons of the popup rows, and their desaturated
/// copies for the widget (`IconUtil.desaturate`).
const PYTHON_ICON: IconName = IconName("python/file");
const VENV_ICON: IconName = IconName("python/virtualEnv");
const UV_ICON: IconName = IconName("python/uv");
const PYTHON_STATUS_ICON: IconName = IconName("python/fileStatus");
const VENV_STATUS_ICON: IconName = IconName("python/virtualEnvStatus");
const UV_STATUS_ICON: IconName = IconName("python/uvStatus");

const PYTHON_SVG: &[u8] = include_bytes!("../../assets/icons/python.svg");
const PYTHON_DARK_SVG: &[u8] = include_bytes!("../../assets/icons/python_dark.svg");
const VENV_SVG: &[u8] = include_bytes!("../../assets/icons/virtualEnv.svg");
const VENV_DARK_SVG: &[u8] = include_bytes!("../../assets/icons/virtualEnv_dark.svg");
const UV_SVG: &[u8] = include_bytes!("../../assets/icons/uv.svg");

static DESATURATED: LazyLock<[Vec<u8>; 5]> = LazyLock::new(|| {
    [PYTHON_SVG, PYTHON_DARK_SVG, VENV_SVG, VENV_DARK_SVG, UV_SVG]
        .map(|svg| desaturate(&String::from_utf8_lossy(svg)).into_bytes())
});

fn icons() -> [IconSvg; 5] {
    let d = &*DESATURATED;
    [
        IconSvg {
            name: VENV_ICON.0,
            light: VENV_SVG,
            dark: Some(VENV_DARK_SVG),
        },
        IconSvg {
            name: UV_ICON.0,
            light: UV_SVG,
            dark: None,
        },
        IconSvg {
            name: PYTHON_STATUS_ICON.0,
            light: &d[0],
            dark: Some(&d[1]),
        },
        IconSvg {
            name: VENV_STATUS_ICON.0,
            light: &d[2],
            dark: Some(&d[3]),
        },
        IconSvg {
            name: UV_STATUS_ICON.0,
            light: &d[4],
            dark: None,
        },
    ]
}

/// Every `#rgb` and `#rrggbb` colour of an SVG as grey: the middle of its
/// lightest and darkest channel (`IconUtil.DesaturationFilter`, the HSL
/// lightness).
fn desaturate(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let hex: String = after.chars().take_while(char::is_ascii_hexdigit).collect();
        let channels = match hex.len() {
            6 => (0..3)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
                .collect::<Option<Vec<u8>>>(),
            3 => hex
                .chars()
                .map(|c| u8::from_str_radix(&format!("{c}{c}"), 16).ok())
                .collect::<Option<Vec<u8>>>(),
            _ => None,
        };
        match channels {
            Some(c) => {
                let max = *c.iter().max().unwrap_or(&0) as u16;
                let min = *c.iter().min().unwrap_or(&0) as u16;
                let grey = ((max + min) / 2) as u8;
                out.push_str(&format!("#{grey:02X}{grey:02X}{grey:02X}"));
                rest = &after[hex.len()..];
            }
            None => {
                out.push('#');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

impl FoundInterpreter {
    /// The flavour icon (`PythonSdkFlavor.getIcon`): uv's for uv
    /// (`UvSdkFlavor`), the virtualenv icon for other virtual environments
    /// (`VirtualEnvSdkFlavor`), the Python file icon for the others.
    pub(crate) fn icon(&self) -> IconName {
        match self.kind {
            _ if self.uv => UV_ICON,
            py_env::InterpreterKind::Venv => VENV_ICON,
            _ => PYTHON_ICON,
        }
    }

    fn status_icon(&self) -> IconName {
        match self.kind {
            _ if self.uv => UV_STATUS_ICON,
            py_env::InterpreterKind::Venv => VENV_STATUS_ICON,
            _ => PYTHON_STATUS_ICON,
        }
    }
}

// ---- registration ----------------------------------------------------------------

/// The live widgets by main window.
#[derive(Default)]
struct Widgets(Vec<(AnyWindowHandle, WeakEntity<InterpreterWidget>)>);

impl Global for Widgets {}

/// The widget of `window`.
fn widget_of(window: AnyWindowHandle, cx: &App) -> Option<Entity<InterpreterWidget>> {
    cx.try_global::<Widgets>()?
        .0
        .iter()
        .filter(|(w, _)| *w == window)
        .find_map(|(_, v)| v.upgrade())
}

/// The widget a command goes to: the active main window's, else the
/// newest.
fn target(cx: &App) -> Option<(AnyWindowHandle, Entity<InterpreterWidget>)> {
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

pub(super) fn init(cx: &mut App) {
    ui::icons::register(cx, &icons());
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ChooseInterpreter, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        // The window may be the one dispatching; open after it.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.open(window, cx));
            });
        });
    });
    cx.on_action(|action: &SelectInterpreter, cx| {
        if let Some((_, widget)) = target(cx) {
            let path = action.path.clone();
            widget.update(cx, |w, cx| w.select(&path, cx));
        }
    });
    ui::command::register_query(
        cx,
        "python.interpreter.state",
        Rc::new(|_, window, cx| {
            let widget = widget_of(window.window_handle(), cx)
                .ok_or_else(|| CommandError::new("not_available", "no interpreter widget"))?;
            Ok(widget.read(cx).state_json(cx))
        }),
    );
}

pub(super) fn status_widget() -> StatusWidget {
    StatusWidget {
        id: "status-interpreter",
        create: Rc::new(|workbench, window, cx| {
            let view = cx.new(|cx| InterpreterWidget::new(workbench, window, cx));
            let handle = window.window_handle();
            let widgets = cx.default_global::<Widgets>();
            widgets.0.retain(|(_, v)| v.upgrade().is_some());
            widgets.0.push((handle, view.downgrade()));
            view.into()
        }),
    }
}

// ---- the widget ------------------------------------------------------------------

/// The interpreter widget of one main window.
pub(super) struct InterpreterWidget {
    root: Option<PathBuf>,
    /// The file of the active editor: the widget shows while it is a file
    /// of the project (`resolvePythonWidgetContext`: the module of the
    /// current file).
    active_file: Option<PathBuf>,
    /// The project's interpreters; `None` while they are read.
    state: Option<ProjectInterpreters>,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Where the widget was drawn (window coordinates), for the popup.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    scan: Task<()>,
    save: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl InterpreterWidget {
    fn new(workbench: WeakEntity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
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
            root: None,
            active_file,
            state: None,
            menu: None,
            bounds: Rc::default(),
            scan: Task::ready(()),
            save: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.set_root(root, cx);
        this
    }

    /// Another project: read its interpreters.
    fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.root = root;
        self.state = None;
        self.menu = None;
        self.rescan(cx);
        cx.notify();
    }

    fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let chosen = super::chosen(&root, cx);
        let scan = cx.background_spawn({
            let root = root.clone();
            async move { super::scan(&root, chosen.as_deref()) }
        });
        self.scan = cx.spawn(async move |this, cx| {
            let state = scan.await;
            let _ = this.update(cx, |this, cx| {
                if this.root.as_ref() == Some(&root) {
                    this.state = Some(state);
                    cx.notify();
                }
            });
        });
    }

    /// Opens the popup over the widget once the project's environments are
    /// read again.
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.menu.is_some() || !self.in_project_file() {
            return;
        }
        let chosen = super::chosen(&root, cx);
        let scan = cx.background_spawn({
            let root = root.clone();
            async move { super::scan(&root, chosen.as_deref()) }
        });
        self.scan = cx.spawn_in(window, async move |this, cx| {
            let state = scan.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.root.as_ref() != Some(&root) {
                    return;
                }
                this.state = Some(state);
                this.show_popup(window, cx);
            });
        });
    }

    fn show_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = &self.state else {
            return;
        };
        let (items, selected) = popup_items(state);
        let options = PopupOptions {
            title: Some(POPUP_TITLE.into()),
            speed_search: true,
            selected,
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    /// Makes `path` (relative to the project or absolute; an interpreter
    /// or its environment folder) the project's interpreter and keeps it.
    fn select(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let path = super::resolve(&root, path);
        // Counts at once (a run started now uses it).
        super::set_chosen(&root, Some(path.clone()), cx);
        let work = cx.background_spawn({
            let root = root.clone();
            async move {
                let state = super::scan(&root, Some(&path));
                if state.chosen
                    && let Some(selected) = &state.selected
                    && let Err(e) = super::store_choice(&root, &selected.python)
                {
                    eprintln!("interpreter: {e}");
                }
                state
            }
        });
        self.save = cx.spawn(async move |this, cx| {
            let state = work.await;
            let _ = this.update(cx, |this, cx| {
                let python = state
                    .chosen
                    .then(|| state.selected.as_ref().map(|s| s.python.clone()))
                    .flatten();
                super::set_chosen(&root, python, cx);
                if this.root.as_ref() == Some(&root) {
                    this.state = Some(state);
                    cx.notify();
                }
            });
            // The language services follow the new interpreter.
            let _ = cx.update(|cx| super::changed(&root, cx));
        });
    }

    /// The active editor shows a file of the project.
    fn in_project_file(&self) -> bool {
        match (&self.root, &self.active_file) {
            (Some(root), Some(file)) => file.starts_with(root),
            _ => false,
        }
    }

    /// The widget's text, tooltip and icon (`PySdkStatusBar.
    /// getWidgetState`); `None` hides it.
    fn presentation(&self) -> Option<(String, Option<String>, Option<IconName>)> {
        if !self.in_project_file() {
            return None;
        }
        let state = self.state.as_ref()?;
        Some(match &state.selected {
            Some(s) => (s.short_name(), Some(s.tooltip()), Some(s.status_icon())),
            None => (NO_INTERPRETER.to_owned(), None, None),
        })
    }

    fn state_json(&self, cx: &App) -> Json {
        let presentation = self.presentation();
        let state = self.state.as_ref();
        json!({
            "visible": presentation.is_some(),
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "text": presentation.as_ref().map(|p| p.0.clone()),
            "tooltip": presentation.as_ref().and_then(|p| p.1.clone()),
            "icon": presentation.as_ref().and_then(|p| p.2).map(|i| i.0),
            "selected": state.and_then(|s| s.selected.as_ref()).map(FoundInterpreter::json),
            "chosen": state.is_some_and(|s| s.chosen),
            "interpreters": state
                .map(|s| s.in_popup_order().into_iter().map(FoundInterpreter::json).collect::<Vec<_>>())
                .unwrap_or_default(),
            "popup": self.menu.as_ref().map(|(m, _)| m.read(cx).state_json()),
        })
    }
}

/// The popup's rows (`PySdkPopupFactory.buildInterpreterActionGroup`) and
/// the current interpreter's row: the interpreters by group with
/// separators, "Add New Interpreter", a separator, "Interpreter
/// Settings…" and "Manage Packages…". The last three come with the
/// interpreter settings and are disabled until then.
fn popup_items(state: &ProjectInterpreters) -> (Vec<MenuItem>, Option<usize>) {
    let mut items = Vec::new();
    let mut selected = None;
    let mut group = None;
    for found in state.in_popup_order() {
        if group.is_some_and(|g: Group| g != found.group()) {
            items.push(MenuItem::separator());
        }
        group = Some(found.group());
        if state
            .selected
            .as_ref()
            .is_some_and(|s| s.python == found.python)
        {
            selected = Some(items.len());
        }
        let action = SelectInterpreter {
            path: found.python.display().to_string(),
        };
        items.push(
            MenuItem::action(found.long_name(), action)
                .icon(found.icon())
                .description(found.description()),
        );
    }
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::submenu(ADD_NEW_INTERPRETER, Vec::new()).enabled(false));
    items.push(MenuItem::separator());
    items.push(MenuItem::unavailable(INTERPRETER_SETTINGS));
    items.push(MenuItem::unavailable(MANAGE_PACKAGES));
    (items, selected)
}

impl Render for InterpreterWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((text, tooltip, icon)) = self.presentation() else {
            return div().into_any_element();
        };
        let hover = cx.theme().color(UiRole::Hover);
        let bounds = self.bounds.clone();
        let overlay = self.menu.as_ref().map(|(menu, _)| {
            ui::menu::overlay_at(menu, self.bounds.get().origin, Corner::BottomLeft, window)
        });
        div()
            .id("status-interpreter")
            .debug_selector(|| "status-interpreter".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            // `TextPanel.WithIconAndArrows`: the icon, a gap of 2, the text.
            .gap(px(2.0))
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .whitespace_nowrap()
            .hover(move |s| s.bg(hover))
            .children(icon.map(Icon::new))
            .child(text)
            .when_some(tooltip, |d, tooltip| {
                d.tooltip(Tooltip::text(tooltip, None))
            })
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
    fn icons_are_desaturated_as_the_reference_does() {
        assert_eq!(
            desaturate(r##"<path fill="#DE5FE9"/><g fill="#fff" clip-path="url(#a)"/>"##),
            r##"<path fill="#A4A4A4"/><g fill="#FFFFFF" clip-path="url(#a)"/>"##
        );
        for icon in icons() {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            if let Some(dark) = icon.dark {
                assert!(ui::icons::rasterize(dark, 32).is_some(), "{}", icon.name);
            }
        }
    }
}
