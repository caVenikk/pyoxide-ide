//! Settings > Python > Interpreter (the reference IDE's
//! `PyActiveSdkConfigurable`): "Python Interpreter:" with the combo of the
//! project's interpreters and the "Add Interpreter" link, above the
//! packages panel (its toolbar and table header; the packages themselves
//! come with the packages tool). A project page: the choice is the
//! person's own for the project (`.oxide/workspace.toml`, ADR 0009).
//!
//! The combo lists "<No interpreter>", the interpreters offered for the
//! project by group (virtual environments first, a separator before the
//! others), a separator and "Show All…". Choosing one changes the page's
//! draft; Apply makes it the project's interpreter. While an interpreter is
//! being configured for the project the combo and the link are disabled;
//! when it is done the page reads the list again.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DismissEvent, Entity,
    FocusHandle, Global, IntoElement, Pixels, Render, SharedString, Subscription, Task, WeakEntity,
    Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::commands::EditField;
use settings_ui::{PageContext, PageSpec, Scope, SettingsPage};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::configuring::{self, BUSY_TOOLTIP};
use super::{Choice, FoundInterpreter, Group, ProjectInterpreters, list};

/// The page's id.
pub const PAGE: &str = "python.interpreter";
/// The reference's texts (PyBundle, IdeBundle, IdeCoreBundle).
const TITLE: &str = "Interpreter";
const LABEL: &str = "Python Interpreter:";
const ADD_LINK: &str = "Add Interpreter";
pub const SHOW_ALL: &str = "Show All\u{2026}";
pub const ADD_LOCAL: &str = "Add Local Interpreter\u{2026}";
pub const NO_INTERPRETER: &str = "<No interpreter>";
const NO_PROJECT: &str = "Open a project to configure its Python interpreter.";
const COLUMNS: [&str; 3] = ["Package", "Version", "Latest version"];
const NOTHING_TO_SHOW: &str = "Nothing to show";
/// The packages toolbar: Install, Uninstall, Upgrade, Show Early Releases.
const TOOLBAR: [(&str, IconName, &str); 4] = [
    ("install", IconName::ADD, "Install"),
    ("uninstall", IconName::REMOVE, "Uninstall"),
    ("upgrade", MOVE_UP_ICON, "Upgrade"),
    ("early-releases", SHOW_ICON, "Show Early Releases"),
];
const SEARCH_TERMS: &[&str] = &[LABEL, ADD_LINK, "Python Interpreters", "Package", "Version"];

/// Icons of the page (New UI icons of the platform).
pub const HOME_FOLDER_ICON: IconName = IconName("interpreter/homeFolder");
const MOVE_UP_ICON: IconName = IconName("interpreter/moveUp");
const SHOW_ICON: IconName = IconName("interpreter/show");
pub(super) const ICONS: [ui::icons::IconSvg; 3] = [
    ui::icons::IconSvg {
        name: HOME_FOLDER_ICON.0,
        light: include_bytes!("../../assets/icons/homeFolder.svg"),
        dark: Some(include_bytes!("../../assets/icons/homeFolder_dark.svg")),
    },
    ui::icons::IconSvg {
        name: MOVE_UP_ICON.0,
        light: include_bytes!("../../assets/icons/moveUp.svg"),
        dark: Some(include_bytes!("../../assets/icons/moveUp_dark.svg")),
    },
    ui::icons::IconSvg {
        name: SHOW_ICON.0,
        light: include_bytes!("../../assets/icons/show.svg"),
        dark: Some(include_bytes!("../../assets/icons/show_dark.svg")),
    },
];

/// Metrics: the gaps of the first row (GridBag insets 2), the toolbar of
/// the packages panel (`ToolbarDecorator`: 16 px icons on a pitch of 26),
/// its table header and the cells' padding.
const ROW_GAP: f32 = 4.0;
const PANEL_GAP: f32 = 8.0;
const TOOLBAR_HEIGHT: f32 = 26.0;
const TOOLBAR_GAP: f32 = 4.0;
const HEADER_HEIGHT: f32 = 25.0;
const CELL_PAD: f32 = 8.0;

/// The page under Python, first (weight 200: Type Engine has 190).
pub fn spec() -> PageSpec {
    PageSpec {
        id: PAGE,
        parent: Some(crate::settings_pages::PYTHON),
        title: TITLE,
        weight: 200,
        scope: Scope::ProjectPersonal,
        search_terms: SEARCH_TERMS,
        description: None,
        create: Some(settings_ui::page::create(InterpreterPage::new)),
    }
}

/// The open pages, for the selection of a new interpreter.
#[derive(Default)]
struct Pages(Vec<WeakEntity<InterpreterPage>>);

impl Global for Pages {}

fn live_pages(cx: &App) -> Vec<Entity<InterpreterPage>> {
    cx.try_global::<Pages>()
        .map(|p| p.0.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

/// The project of an open Interpreter page.
pub(crate) fn open_project(cx: &App) -> Option<PathBuf> {
    live_pages(cx)
        .into_iter()
        .find_map(|p| p.read(cx).root.clone())
}

/// Selects a new interpreter in the open pages of the project at `root`
/// (`select: "draft"`): the page reads the list again and shows it.
pub(crate) fn select_draft(root: &Path, python: &Path, cx: &mut App) {
    for page in live_pages(cx) {
        page.update(cx, |page, cx| {
            if page.root.as_deref() == Some(root) {
                page.draft = Some(Choice::Python(python.to_owned()));
                page.rescan(cx);
                cx.notify();
            }
        });
    }
}

/// The open list of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    /// The combo's interpreters.
    Interpreters,
    /// The "Add Interpreter" popup.
    Add,
}

impl List {
    fn id(self) -> &'static str {
        match self {
            List::Interpreters => "interpreters",
            List::Add => "add",
        }
    }

    fn index(self) -> usize {
        match self {
            List::Interpreters => 0,
            List::Add => 1,
        }
    }
}

/// A row of the combo's list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    NoInterpreter,
    Interpreter(FoundInterpreter),
    Separator,
    ShowAll,
}

/// The combo's rows (`updateSdkListAndSelect`): "<No interpreter>", the
/// groups in their order with a separator before every group but the
/// first kind, a separator and "Show All…".
fn rows(state: &ProjectInterpreters) -> Vec<Row> {
    let mut rows = vec![Row::NoInterpreter];
    let mut group = None;
    for found in state.in_popup_order() {
        if group != Some(found.group()) {
            group = Some(found.group());
            if found.group() != Group::VirtualEnv {
                rows.push(Row::Separator);
            }
        }
        rows.push(Row::Interpreter(found.clone()));
    }
    rows.push(Row::Separator);
    rows.push(Row::ShowAll);
    rows
}

/// The grey text after an interpreter's name: its version and the path of
/// its executable (`customizeWithSdkValue`).
fn detail(found: &FoundInterpreter) -> String {
    let path = found.python.display().to_string();
    match &found.version {
        Some(version) => format!("{version} {path}"),
        None => path,
    }
}

/// Python > Interpreter.
pub struct InterpreterPage {
    root: Option<PathBuf>,
    /// The project's interpreters; `None` while they are read.
    state: Option<ProjectInterpreters>,
    /// The combo's selection, not applied yet.
    draft: Option<Choice>,
    /// An interpreter is being configured for the project.
    configuring: bool,
    list: Option<(List, Entity<ContextMenu>, Subscription)>,
    /// Where the combo and the link are drawn.
    anchors: Rc<RefCell<[Bounds<Pixels>; 2]>>,
    error: Option<String>,
    focus: FocusHandle,
    scan: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl InterpreterPage {
    pub fn new(context: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        list::ensure_loaded(cx);
        let root = context.project.clone();
        let configuring = root
            .as_deref()
            .is_some_and(|r| configuring::is_configuring(r, cx));
        let subscriptions = vec![
            cx.observe_global::<list::Interpreters>(|this, cx| this.rescan(cx)),
            cx.observe_global::<configuring::Configuring>(|this, cx| {
                let busy = this
                    .root
                    .as_deref()
                    .is_some_and(|r| configuring::is_configuring(r, cx));
                if busy != this.configuring {
                    this.configuring = busy;
                    if busy {
                        this.close_list(cx);
                    } else {
                        // `onSdkConfigurationFinished`: read again.
                        this.rescan(cx);
                    }
                    cx.notify();
                }
            }),
        ];
        let weak = cx.entity().downgrade();
        let pages = cx.default_global::<Pages>();
        pages.0.retain(|p| p.upgrade().is_some());
        pages.0.push(weak);
        let mut this = Self {
            root,
            state: None,
            draft: None,
            configuring,
            list: None,
            anchors: Rc::default(),
            error: None,
            focus: cx.focus_handle(),
            scan: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.rescan(cx);
        this
    }

    /// Reads the project's interpreters again (off the UI thread).
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let chosen = super::chosen(&root, cx);
        let table = list::table(cx);
        let scan = cx.background_spawn({
            let root = root.clone();
            async move { super::scan(&root, chosen.as_ref(), &table) }
        });
        self.scan = cx.spawn(async move |this, cx| {
            let state = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.state = Some(state);
                cx.notify();
            });
        });
    }

    /// The project's interpreter as stored.
    fn stored(&self) -> Option<Choice> {
        let state = self.state.as_ref()?;
        Some(match &state.selected {
            Some(found) => Choice::Python(found.python.clone()),
            None => Choice::None,
        })
    }

    /// The combo's selection.
    fn selection(&self) -> Option<Choice> {
        self.draft.clone().or_else(|| self.stored())
    }

    /// The interpreter the combo shows (one not read yet by its path).
    fn selected_found(&self) -> Option<FoundInterpreter> {
        let Some(Choice::Python(python)) = self.selection() else {
            return None;
        };
        self.state
            .as_ref()
            .and_then(|s| s.found.iter().find(|f| f.python == python).cloned())
            .or_else(|| Some(FoundInterpreter::bare(&python)))
    }

    fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        self.draft = Some(choice);
        self.close_list(cx);
        cx.notify();
    }

    fn items(&self, list: List) -> (Vec<MenuItem>, Option<usize>) {
        match list {
            List::Interpreters => {
                let Some(state) = &self.state else {
                    return (Vec::new(), None);
                };
                let selection = self.selection();
                let mut selected = None;
                let edit = |value: Json| EditField {
                    field: "interpreter".into(),
                    value,
                };
                let items = rows(state)
                    .into_iter()
                    .enumerate()
                    .map(|(ix, row)| match row {
                        Row::NoInterpreter => {
                            if selection == Some(Choice::None) {
                                selected = Some(ix);
                            }
                            MenuItem::action(NO_INTERPRETER, edit(Json::Null))
                        }
                        Row::Interpreter(found) => {
                            if selection == Some(Choice::Python(found.python.clone())) {
                                selected = Some(ix);
                            }
                            MenuItem::action(
                                found.name.clone(),
                                edit(json!(found.python.display().to_string())),
                            )
                            .icon(found.icon())
                            .secondary(detail(&found))
                        }
                        Row::Separator => MenuItem::separator(),
                        // The Python Interpreters dialog comes with the
                        // list of interpreters (#75, stage 3).
                        Row::ShowAll => MenuItem::unavailable(SHOW_ALL),
                    })
                    .collect();
                (items, selected)
            }
            // The Add Python Interpreter dialog (#75, stage 2).
            List::Add => (
                vec![MenuItem::unavailable(ADD_LOCAL).icon(HOME_FOLDER_ICON)],
                None,
            ),
        }
    }

    fn open_list(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        if self.configuring || self.state.is_none() {
            return;
        }
        let (items, selected) = self.items(list);
        let bounds = self.anchors.borrow()[list.index()];
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                speed_search: true,
                selected,
                min_width: (list == List::Interpreters).then(|| f32::from(bounds.size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((list, menu, subscription));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_list(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.as_ref().is_some_and(|(l, _, _)| *l == list) {
            self.close_list(cx);
        } else {
            self.open_list(list, window, cx);
        }
    }

    // ---- drawing ----

    fn anchor(&self, list: List) -> impl IntoElement {
        let anchors = self.anchors.clone();
        canvas(
            move |bounds, _, _| anchors.borrow_mut()[list.index()] = bounds,
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// The combo's content: the selected interpreter's icon, name, version
    /// and path, or "<No interpreter>".
    fn combo_content(&self, cx: &App) -> Vec<AnyElement> {
        let theme = cx.theme();
        let small = theme.ui_size() - px(2.0);
        let muted = theme.color(UiRole::TextMuted);
        match self.selected_found() {
            None if self.state.is_none() => Vec::new(),
            None => vec![div().child(NO_INTERPRETER).into_any_element()],
            Some(found) => {
                let mut out = vec![
                    Icon::new(found.icon()).into_any_element(),
                    div()
                        .flex_none()
                        .child(found.name.clone())
                        .into_any_element(),
                ];
                if let Some(version) = &found.version {
                    out.push(
                        div()
                            .flex_none()
                            .text_size(small)
                            .text_color(muted)
                            .child(version.clone())
                            .into_any_element(),
                    );
                }
                out.push(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(small)
                        .text_color(muted)
                        .child(found.python.display().to_string())
                        .into_any_element(),
                );
                out
            }
        }
    }

    fn combo(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = !self.configuring && self.state.is_some();
        let open = self
            .list
            .as_ref()
            .is_some_and(|(l, _, _)| *l == List::Interpreters);
        let focus = cx.theme().color(UiRole::Focus);
        ui::form::combo("interpreter-combo", self.combo_content(cx), cx)
            .debug_selector(|| "interpreter:combo".into())
            .flex_1()
            .min_w(px(0.0))
            .when(open, |d| d.border_color(focus))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(self.configuring, |d| {
                d.tooltip(Tooltip::text(BUSY_TOOLTIP, None))
            })
            .when(enabled, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_list(List::Interpreters, window, cx)
                }))
            })
            .child(self.anchor(List::Interpreters))
            .into_any_element()
    }

    fn link(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = !self.configuring;
        let link = ui::link::link("interpreter-add-link", ADD_LINK, cx)
            .debug_selector(|| "interpreter:add-link".into())
            .relative()
            .flex_none()
            .child(Icon::new(IconName::CHEVRON_DOWN));
        let link = if enabled {
            link.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_list(List::Add, window, cx)
            }))
        } else {
            // `setAutoHideOnDisable(false)`: shown, greyed.
            link.text_color(cx.theme().color(UiRole::TextMuted))
                .tooltip(Tooltip::text(BUSY_TOOLTIP, None))
        };
        link.child(self.anchor(List::Add)).into_any_element()
    }

    /// The packages panel: the toolbar (disabled until the packages tool
    /// fills the table), the header and the empty table.
    fn packages(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let toolbar = div()
            .flex_none()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(TOOLBAR_GAP))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(TOOLBAR_GAP))
            .children(TOOLBAR.iter().map(|(id, icon, tooltip)| {
                let id = *id;
                ui::components::small_icon_button(
                    SharedString::from(format!("interpreter-packages-{id}")),
                    *icon,
                    false,
                    cx,
                )
                .debug_selector(move || format!("interpreter:packages:{id}"))
                .tooltip(Tooltip::text(*tooltip, None))
            }));
        let header = div()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .bg(theme.color(UiRole::Surface))
            .border_b_1()
            .border_color(border)
            .children(COLUMNS.iter().enumerate().map(|(ix, title)| {
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .px(px(CELL_PAD))
                    .when(ix > 0, |d| d.border_l_1().border_color(border))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(*title)
            }));
        let body = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.color(UiRole::TextMuted))
            .child(NOTHING_TO_SHOW);
        div()
            .debug_selector(|| "interpreter:packages".into())
            .flex_1()
            .min_h(px(TOOLBAR_HEIGHT + HEADER_HEIGHT * 3.0))
            .flex()
            .flex_col()
            .border_1()
            .border_color(border)
            .child(toolbar)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn row_json(row: &Row, selection: Option<&Choice>) -> Json {
        match row {
            Row::NoInterpreter => json!({
                "kind": "none",
                "text": NO_INTERPRETER,
                "selected": selection == Some(&Choice::None),
            }),
            Row::Interpreter(found) => json!({
                "kind": "interpreter",
                "name": found.name,
                "version": found.version,
                "path": found.python.display().to_string(),
                "icon": found.icon().0,
                "group": match found.group() {
                    Group::VirtualEnv => "virtualenv",
                    Group::System => "system",
                },
                "configured": found.configured,
                "selected": selection == Some(&Choice::Python(found.python.clone())),
            }),
            Row::Separator => json!({"kind": "separator"}),
            Row::ShowAll => json!({"kind": "show-all", "text": SHOW_ALL}),
        }
    }
}

impl SettingsPage for InterpreterPage {
    fn is_modified(&self, _: &App) -> bool {
        self.draft.is_some() && self.draft != self.stored()
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let (Some(root), Some(choice)) = (self.root.clone(), self.draft.take()) else {
            return Ok(());
        };
        if Some(&choice) == self.stored().as_ref() {
            return Ok(());
        }
        // The choice outlives the page (OK closes the window at once).
        let chosen = super::choose(&root, choice, cx);
        cx.spawn(async move |this, cx| {
            let state = chosen.await;
            let _ = this.update(cx, |this, cx| {
                this.state = Some(state);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = None;
        self.error = None;
        self.close_list(cx);
        self.rescan(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let selection = self.selection();
        let rows = self.state.as_ref().map(rows).unwrap_or_default();
        let enabled = !self.configuring;
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "loaded": self.state.is_some(),
            "list-loaded": list::is_loaded(cx),
            "label": LABEL,
            "items": rows.iter().map(|r| Self::row_json(r, selection.as_ref())).collect::<Vec<_>>(),
            "selected": match &selection {
                None => Json::Null,
                Some(Choice::None) => json!({"kind": "none", "text": NO_INTERPRETER}),
                Some(Choice::Python(_)) => self.selected_found().map(|f| json!({
                    "kind": "interpreter",
                    "name": f.name,
                    "version": f.version,
                    "path": f.python.display().to_string(),
                })).unwrap_or(Json::Null),
            },
            "configuring": self.configuring,
            "combo-enabled": enabled && self.state.is_some(),
            "link": ADD_LINK,
            "link-enabled": enabled,
            "tooltip": (!enabled).then_some(BUSY_TOOLTIP),
            "toolbar": TOOLBAR.iter().map(|(id, _, tooltip)| json!({"id": id, "tooltip": tooltip, "enabled": false})).collect::<Vec<_>>(),
            "columns": COLUMNS,
            "empty-text": NOTHING_TO_SHOW,
            "list": self.list.as_ref().map(|(l, _, _)| l.id()),
            "menu": self.list.as_ref().map(|(_, m, _)| m.read(cx).state_json()),
            "error": self.error,
        })
    }

    /// Fields: `interpreter` (an interpreter's path, or null for "<No
    /// interpreter>"), `list` (`interpreters` opens the combo's list,
    /// `add` the "Add Interpreter" popup; null closes it).
    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let root = self.root.clone().ok_or(NO_PROJECT)?;
        match field {
            "interpreter" => {
                if self.configuring {
                    return Err(BUSY_TOOLTIP.into());
                }
                let choice = match value {
                    Json::Null => Choice::None,
                    Json::String(path) => Choice::Python(super::resolve(&root, path)),
                    _ => return Err("`interpreter` takes a path or null".into()),
                };
                self.choose(choice, cx);
                Ok(())
            }
            "list" => {
                match value.as_str() {
                    Some("interpreters") => self.open_list(List::Interpreters, window, cx),
                    Some("add") => self.open_list(List::Add, window, cx),
                    None => {
                        self.close_list(cx);
                        window.focus(&self.focus);
                    }
                    Some(other) => return Err(format!("no list `{other}`")),
                }
                Ok(())
            }
            _ => Err(format!("no field `{field}` on this page")),
        }
    }
}

impl Render for InterpreterPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = div()
            .id("interpreter-page")
            .debug_selector(|| "interpreter-page".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(PANEL_GAP));
        if self.root.is_none() {
            return page.child(
                div()
                    .text_color(cx.theme().color(UiRole::TextMuted))
                    .child(NO_PROJECT),
            );
        }
        let row = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROW_GAP))
            .child(
                div()
                    .flex_none()
                    .child(settings_ui::search::label(LABEL, cx)),
            )
            .child(self.combo(cx))
            .child(self.link(cx));
        let overlay = self.list.as_ref().map(|(list, menu, _)| {
            let at = self.anchors.borrow()[list.index()].bottom_left();
            ui::menu::overlay(menu, at, window)
        });
        page.child(row)
            .child(self.packages(cx))
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(cx.theme().color(UiRole::Danger)).child(e)),
            )
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use py_env::InterpreterKind;

    fn found(name: &str, kind: InterpreterKind, uv: bool) -> FoundInterpreter {
        let mut f = FoundInterpreter::bare(Path::new(&format!("/x/{name}/bin/python")));
        f.name = name.into();
        f.kind = kind;
        f.uv = uv;
        f.version = Some("3.13.1".into());
        f
    }

    fn kinds(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::NoInterpreter => "none".to_owned(),
                Row::Interpreter(f) => f.name.clone(),
                Row::Separator => "-".to_owned(),
                Row::ShowAll => "all".to_owned(),
            })
            .collect()
    }

    /// The combo's rows as `updateSdkListAndSelect` builds them.
    #[test]
    fn rows_by_group_with_the_reference_separators() {
        let empty = ProjectInterpreters::default();
        assert_eq!(kinds(&rows(&empty)), ["none", "-", "all"]);
        let venvs = ProjectInterpreters {
            found: vec![
                found("b", InterpreterKind::Venv, false),
                found("a", InterpreterKind::Conda, false),
            ],
            ..ProjectInterpreters::default()
        };
        // Virtual environments follow "<No interpreter>" directly.
        assert_eq!(kinds(&rows(&venvs)), ["none", "a", "b", "-", "all"]);
        let mixed = ProjectInterpreters {
            found: vec![
                found("uv", InterpreterKind::Venv, true),
                found("venv", InterpreterKind::Venv, false),
                found("Python 3.14", InterpreterKind::System, false),
            ],
            ..ProjectInterpreters::default()
        };
        assert_eq!(
            kinds(&rows(&mixed)),
            ["none", "venv", "-", "Python 3.14", "uv", "-", "all"]
        );
        // The others always get a separator.
        let system = ProjectInterpreters {
            found: vec![found("uv", InterpreterKind::Venv, true)],
            ..ProjectInterpreters::default()
        };
        assert_eq!(kinds(&rows(&system)), ["none", "-", "uv", "-", "all"]);
        assert_eq!(
            detail(&system.found[0]),
            format!("3.13.1 {}", Path::new("/x/uv/bin/python").display())
        );
    }
}
