//! Python > Debugger (`PyDebuggerConfigurable`), the Python debugger options
//! of the project (`py_debug::settings`: `[python.debugger]` of
//! `.oxide/workspace.toml`, picked up once from the reference's
//! `.idea/workspace.xml`). The page reads them off the UI thread when it
//! opens and writes them off it on Apply; the next debug session starts
//! with them.
//!
//! What debugpy does is wired: "Attach to subprocess automatically while
//! debugging", "Gevent compatible", "PyQt compatible" with its backend and
//! "Debugger evaluation response timeout (ms)". Drawn disabled as the
//! reference has them: "Collect run-time types information for code
//! insight" with its warning and "Clear caches", "Drop into debugger on
//! failed tests", the filter of Attach To Process, and pydevd in "Mode".
//!
//! Not drawn: "Run debugger in server mode" and its port (a registry
//! option of the reference, off by default), and the up and down arrow keys
//! of the timeout's spinner.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DismissEvent, Div, ElementId,
    Entity, FocusHandle, FontWeight, HighlightStyle, IntoElement, Pixels, Render, SharedString,
    Stateful, StyledText, Subscription, Task, Window, canvas, div, prelude::*, px,
};
use py_debug::settings::{self, DebuggerSettings, MAX_TIMEOUT_MS, QtBackend};
use serde_json::{Value as Json, json};
use settings_ui::commands::EditField;
use settings_ui::{PageContext, SettingsPage};
use text::{Selection, SelectionSet};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip, checkbox_with};
use ui::form::CHECK_GAP;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

/// The reference's texts (PyBundle).
const ATTACH_TO_SUBPROCESS: &str = "Attach to subprocess automatically while debugging";
const COLLECT_TYPES: &str = "Collect run-time types information for code insight";
const CLEAR_CACHES: &str = "Clear caches";
const SLOW_WARNING: &str = "This option may slow down the debugger";
const GEVENT: &str = "Gevent compatible";
const DROP_INTO_DEBUGGER: &str = "Drop into debugger on failed tests";
const PYQT: &str = "PyQt compatible";
const ATTACH_FILTER: &str = "For Attach To Process show processes with names containing:";
/// The part of [`ATTACH_FILTER`] the reference sets in bold.
const ATTACH_FILTER_BOLD: &str = "Attach To Process";
const ATTACH_FILTER_DEFAULT: &str = "python";
const TIMEOUT: &str = "Debugger evaluation response timeout (ms):";
const MODE: &str = "Mode:";
const PYDEVD: &str = "pydevd";
const DEBUGPY: &str = "debugpy";
const DEBUGPY_DESCRIPTION: &str = "Python debugger backend built on the Debug Adapter Protocol, with a more stable connection model. Lower overhead on Python 3.12+.";
/// Pyoxide's own.
const NOT_SUPPORTED: &str = "Not supported yet";
const NO_PROJECT: &str = "Open a project to configure its Python debugger.";

/// The words the search finds the page by.
pub const SEARCH_TERMS: &[&str] = &[
    ATTACH_TO_SUBPROCESS,
    COLLECT_TYPES,
    CLEAR_CACHES,
    GEVENT,
    DROP_INTO_DEBUGGER,
    PYQT,
    ATTACH_FILTER,
    TIMEOUT,
    MODE,
    PYDEVD,
    DEBUGPY,
    DEBUGPY_DESCRIPTION,
];

/// The page's rows as the reference's screenshot spaces them (measured
/// against its text width, not taken from the sources): check box rows 22
/// high 7.5 apart, rows with a field, combo box, spinner or segmented
/// button 24 high with 11 between the blocks and 12 between rows of
/// fields, 7 between a label and its field, the combo box and the spinner
/// as wide as the reference's.
const CHECK_ROW: f32 = 22.0;
const CHECK_GAP_V: f32 = 7.5;
const FIELD_ROW: f32 = 24.0;
const BLOCK_GAP: f32 = 11.0;
const FIELD_GAP: f32 = 12.0;
const HINT_GAP: f32 = 10.5;
/// The space above the first row, and the extra space above Mode.
const PAGE_TOP: f32 = 4.0;
const MODE_TOP: f32 = 2.0;
const LABEL_GAP: f32 = 7.0;
const MODE_GAP: f32 = 13.0;
const COMBO_WIDTH: f32 = 88.0;
const SPINNER_WIDTH: f32 = 142.0;
const SPINNER_STEP: u32 = 500;
const ICON_GAP: f32 = 12.0;
/// The spinner's arrow buttons, each: width and height.
const SPINNER_ARROW: (f32, f32) = (20.0, 13.0);

/// How a one-line field of the page is drawn.
#[derive(Clone, Copy)]
enum FieldKind {
    /// Not supported yet: dim, takes no clicks.
    Disabled,
    /// The timeout's spinner; `error`: its text is no number in range.
    Spinner { error: bool },
}

/// The keys of `settings.edit`.
mod field {
    pub const ATTACH_TO_SUBPROCESS: &str = "attach-to-subprocess";
    pub const GEVENT: &str = "gevent";
    pub const PYQT: &str = "pyqt";
    pub const PYQT_BACKEND: &str = "pyqt-backend";
    pub const TIMEOUT: &str = "evaluation-timeout";
    /// Opens (`true`) or closes the PyQt backend's list.
    pub const LIST: &str = "list";
}

/// Python > Debugger.
pub struct DebuggerPage {
    root: Option<PathBuf>,
    /// The stored options; `None` until they are read.
    stored: Option<DebuggerSettings>,
    /// The draft: the check boxes and the backend (the timeout is the
    /// field's text).
    draft: DebuggerSettings,
    timeout: Entity<Editor>,
    /// "python", drawn disabled.
    attach_filter: Entity<Editor>,
    /// The open list of the PyQt backends.
    list: Option<(Entity<ContextMenu>, Subscription)>,
    /// Where the PyQt combo box is drawn.
    anchor: Rc<Cell<Bounds<Pixels>>>,
    error: Option<String>,
    focus: FocusHandle,
    _read: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl DebuggerPage {
    pub fn new(context: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let defaults = DebuggerSettings::default();
        let timeout = editor::field::new(
            &defaults.evaluation_timeout_ms.to_string(),
            None,
            true,
            window,
            cx,
        );
        let attach_filter = editor::field::new(ATTACH_FILTER_DEFAULT, None, true, window, cx);
        let subscriptions = vec![cx.subscribe(
            &timeout,
            |_: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
                if matches!(e, EditorEvent::Edited(_)) {
                    cx.notify();
                }
            },
        )];
        let mut this = Self {
            root: context.project.clone(),
            stored: None,
            draft: defaults,
            timeout,
            attach_filter,
            list: None,
            anchor: Rc::default(),
            error: None,
            focus: cx.focus_handle(),
            _read: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.read(cx);
        this
    }

    /// Reads the options in the background; the draft becomes them.
    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let work = cx.background_spawn(async move { settings::load(&root) });
        self._read = cx.spawn(async move |this, cx| {
            let loaded = work.await;
            let _ = this.update(cx, |this, cx| {
                match loaded {
                    Ok(settings) => this.show(settings, cx),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        });
    }

    /// Makes `settings` the stored options and the draft.
    fn show(&mut self, settings: DebuggerSettings, cx: &mut Context<Self>) {
        let text = settings.evaluation_timeout_ms.to_string();
        self.put(&text, cx);
        self.draft = settings.clone();
        self.stored = Some(settings);
        self.error = None;
    }

    /// Puts `text` into the timeout's field, the caret at its end.
    fn put(&self, text: &str, cx: &mut Context<Self>) {
        let end = text.chars().count();
        self.timeout.update(cx, |e, cx| {
            e.set_text_selected(text, cx);
            e.set_selections(SelectionSet::single(Selection::new(end, end)), cx);
        });
    }

    fn text(&self, cx: &App) -> String {
        self.timeout.read(cx).text().trim().to_owned()
    }

    /// The timeout the field holds, when it is a number in range.
    fn timeout_ms(&self, cx: &App) -> Result<u32, String> {
        self.text(cx)
            .parse::<u32>()
            .ok()
            .filter(|n| *n <= MAX_TIMEOUT_MS)
            .ok_or_else(|| format!("Please enter a number from 0 to {MAX_TIMEOUT_MS}"))
    }

    /// The draft as options.
    fn current(&self, cx: &App) -> Result<DebuggerSettings, String> {
        Ok(DebuggerSettings {
            evaluation_timeout_ms: self.timeout_ms(cx)?,
            ..self.draft.clone()
        })
    }

    fn set_timeout(&mut self, value: u32, cx: &mut Context<Self>) {
        self.put(&value.to_string(), cx);
        cx.notify();
    }

    /// The up and down arrows of the spinner: a step of 500 from the number
    /// in the field (the stored one when the field holds none).
    fn step(&mut self, up: bool, cx: &mut Context<Self>) {
        let now = self.timeout_ms(cx).unwrap_or_else(|_| {
            self.stored
                .as_ref()
                .map_or(settings::DEFAULT_TIMEOUT_MS, |s| s.evaluation_timeout_ms)
        });
        let next = if up {
            now.saturating_add(SPINNER_STEP).min(MAX_TIMEOUT_MS)
        } else {
            now.saturating_sub(SPINNER_STEP)
        };
        self.set_timeout(next, cx);
    }

    fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            field::ATTACH_TO_SUBPROCESS => {
                self.draft.attach_to_subprocess = !self.draft.attach_to_subprocess;
            }
            field::GEVENT => self.draft.gevent = !self.draft.gevent,
            field::PYQT => {
                self.draft.pyqt = !self.draft.pyqt;
                self.list = None;
            }
            _ => return,
        }
        cx.notify();
    }

    fn choose(&mut self, backend: QtBackend, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.pyqt_backend = backend;
        self.close_list_in(window, cx);
        cx.notify();
    }

    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        if !self.draft.pyqt {
            return;
        }
        let items = QtBackend::ALL
            .iter()
            .map(|backend| {
                MenuItem::action(
                    backend.name(),
                    EditField {
                        field: field::PYQT_BACKEND.to_owned(),
                        value: json!(backend.id()),
                    },
                )
                .enabled(backend.debugpy_supports())
            })
            .collect();
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                selected: QtBackend::ALL
                    .iter()
                    .position(|b| *b == self.draft.pyqt_backend),
                min_width: Some(f32::from(self.anchor.get().size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((menu, subscription));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            cx.notify();
        }
    }

    /// Closes the list and gives the focus back to the page (the list had
    /// it).
    fn close_list_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.is_some() {
            self.close_list(cx);
            window.focus(&self.focus);
        }
    }

    // ---- drawing ----

    /// A check box with its label, lit up while the window's search finds
    /// the label. A row that is not `enabled` is drawn disabled and takes no
    /// clicks.
    fn check(
        &self,
        name: &'static str,
        checked: bool,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let color = theme.color(if enabled {
            UiRole::Text
        } else {
            UiRole::TextDisabled
        });
        let row = div()
            .id(ElementId::Name(format!("debugger-row:{name}").into()))
            .debug_selector(move || format!("debugger:{name}"))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(CHECK_GAP))
            .h(px(CHECK_ROW))
            .child(checkbox_with(
                ElementId::Name(format!("debugger-check:{name}").into()),
                checked,
                enabled,
                cx,
            ))
            .child(
                div()
                    .whitespace_nowrap()
                    .text_color(color)
                    .child(SharedString::from(label)),
            );
        let row = if enabled {
            row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(name, cx)))
                .into_any_element()
        } else {
            row.tooltip(Tooltip::text(NOT_SUPPORTED, None))
                .into_any_element()
        };
        settings_ui::search::spotlight(label, row, cx)
    }

    /// A one-line field in its frame.
    fn framed(
        &self,
        id: &'static str,
        field: &Entity<Editor>,
        kind: FieldKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (enabled, error, spinner) = match kind {
            FieldKind::Disabled => (false, false, false),
            FieldKind::Spinner { error } => (true, error, true),
        };
        let theme = cx.theme();
        let mut frame = editor::field::frame(field, true, error, window, cx);
        if spinner {
            let border = theme.color(UiRole::Border);
            let color = theme.color(UiRole::Text);
            // The reference's spinner arrows: small solid triangles.
            let arrow =
                |id: &'static str, glyph: &'static str, up: bool, cx: &mut Context<Self>| {
                    div()
                        .id(id)
                        .debug_selector(move || format!("debugger:timeout-{id}"))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(SPINNER_ARROW.0))
                        .h(px(SPINNER_ARROW.1))
                        .text_size(px(7.0))
                        .text_color(color)
                        .pointing_hand()
                        .child(glyph)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.step(up, cx)))
                };
            frame = frame.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .border_l_1()
                    .border_color(border)
                    .child(arrow("up", "\u{25B2}", true, cx))
                    .child(arrow("down", "\u{25BC}", false, cx)),
            );
        }
        div()
            .id(id)
            .relative()
            .flex_none()
            .h(px(FIELD_ROW))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .child(frame.h(px(FIELD_ROW)))
            // A disabled field takes no clicks.
            .when(!enabled, |d| {
                d.child(
                    div()
                        .id((id, 1usize))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude()
                        .tooltip(Tooltip::text(NOT_SUPPORTED, None)),
                )
            })
    }

    /// The "PyQt compatible" row: the check box and the backend's combo
    /// box, which goes with it.
    fn qt_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let enabled = self.draft.pyqt;
        let anchor = self.anchor.clone();
        let open = self.list.is_some();
        let combo = ui::form::combo(
            "debugger-qt-backend",
            vec![
                div()
                    .child(self.draft.pyqt_backend.name())
                    .into_any_element(),
            ],
            cx,
        )
        .debug_selector(|| "debugger:qt-backend".into())
        .w(px(COMBO_WIDTH))
        .h(px(FIELD_ROW))
        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
        .when(enabled, |d| {
            d.pointing_hand()
                .when(open, |d| d.bg(theme.color(UiRole::Hover)))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    if this.list.is_some() {
                        this.close_list_in(window, cx);
                    } else {
                        this.open_list(window, cx);
                    }
                }))
        })
        .child(
            canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ICON_GAP))
            .h(px(FIELD_ROW))
            .child(self.check(field::PYQT, self.draft.pyqt, PYQT, true, cx))
            .child(combo)
            .into_any_element()
    }

    /// "Collect run-time types information for code insight", the warning
    /// that it slows the debugger down, and the link that clears what was
    /// collected: not supported yet.
    fn collect_types_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let link = div()
            .id("debugger-clear-caches")
            .debug_selector(|| "debugger:clear-caches".into())
            .text_color(theme.color(UiRole::TextLink))
            .opacity(DISABLED_ALPHA)
            .child(CLEAR_CACHES)
            .tooltip(Tooltip::text(NOT_SUPPORTED, None));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ICON_GAP))
            .child(self.check("collect-types", false, COLLECT_TYPES, false, cx))
            .child(
                div()
                    .id("debugger-slow-warning")
                    .flex_none()
                    .opacity(DISABLED_ALPHA)
                    .child(Icon::new(IconName::WARNING))
                    .tooltip(Tooltip::text(SLOW_WARNING, None)),
            )
            .child(settings_ui::search::spotlight(CLEAR_CACHES, link, cx))
            .into_any_element()
    }

    /// A label of a form row: text that does not wrap, lit up while the
    /// search finds it; `bold` is part of it in bold.
    fn label(
        text: &'static str,
        bold: Option<&'static str>,
        enabled: bool,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        let color = theme.color(if enabled {
            UiRole::Text
        } else {
            UiRole::TextDisabled
        });
        let styled = match bold.and_then(|b| text.find(b).map(|at| at..at + b.len())) {
            Some(range) => StyledText::new(text).with_highlights([(
                range,
                HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..HighlightStyle::default()
                },
            )]),
            None => StyledText::new(text),
        };
        let element = div().whitespace_nowrap().text_color(color).child(styled);
        settings_ui::search::spotlight(text, element, cx)
    }

    /// "Mode": pydevd and debugpy side by side; the page runs debugpy only.
    fn mode_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices: Vec<SharedString> = vec![PYDEVD.into(), DEBUGPY.into()];
        let selector = ui::form::segmented_enabled(
            "debugger-mode",
            &choices,
            Some(1),
            &[false, true],
            |_, _, _| {},
            cx,
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(MODE_GAP))
            .h(px(FIELD_ROW))
            .child(Self::label(MODE, None, true, cx))
            .child(
                div()
                    .id("debugger-mode-selector")
                    .child(selector.h(px(FIELD_ROW)))
                    .tooltip(Tooltip::text("pydevd is not supported yet", None)),
            )
            .into_any_element()
    }
}

impl SettingsPage for DebuggerPage {
    fn is_modified(&self, cx: &App) -> bool {
        match (&self.stored, self.current(cx)) {
            (Some(stored), Ok(current)) => *stored != current,
            (Some(_), Err(_)) => true,
            (None, _) => false,
        }
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let (Some(root), Some(stored)) = (self.root.clone(), &self.stored) else {
            return Ok(());
        };
        let current = self.current(cx)?;
        if *stored == current {
            return Ok(());
        }
        self.stored = Some(current.clone());
        self.error = None;
        let work = cx.background_spawn(async move {
            settings::save(&root, &current).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = work.await {
                eprintln!("python debugger settings: {e}");
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(e);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.list = None;
        match self.stored.clone() {
            Some(stored) => self.show(stored, cx),
            None => self.read(cx),
        }
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let timeout = self.timeout_ms(cx);
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "loaded": self.stored.is_some(),
            "attach-to-subprocess": self.draft.attach_to_subprocess,
            "collect-types": {"checked": false, "enabled": false},
            "gevent": self.draft.gevent,
            "drop-into-debugger": {"checked": false, "enabled": false},
            "pyqt": self.draft.pyqt,
            "pyqt-backend": self.draft.pyqt_backend.id(),
            "pyqt-backends": QtBackend::ALL.map(|b| json!({
                "id": b.id(),
                "name": b.name(),
                "enabled": b.debugpy_supports(),
            })),
            "pyqt-backend-enabled": self.draft.pyqt,
            "attach-filter": {"text": ATTACH_FILTER_DEFAULT, "enabled": false},
            "evaluation-timeout": self.text(cx),
            "evaluation-timeout-valid": timeout.is_ok(),
            "mode": {
                "choices": [PYDEVD, DEBUGPY],
                "selected": DEBUGPY,
                "enabled": [false, true],
                "description": DEBUGPY_DESCRIPTION,
            },
            "list": self.list.as_ref().map(|(m, _)| m.read(cx).state_json()),
            "error": self.error.clone(),
        })
    }

    /// Fields: `attach-to-subprocess`, `gevent`, `pyqt` (true or false),
    /// `pyqt-backend` (`auto`, `pyqt4`, `pyqt5`, `pyside`, `pyside2`),
    /// `evaluation-timeout` (a number, or a text as typed), `list` (true
    /// opens the PyQt backends' list, null closes it).
    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.stored.is_none() {
            return Err("the settings are not read yet".into());
        }
        let flag = || {
            value
                .as_bool()
                .ok_or_else(|| format!("`{name}` takes true or false"))
        };
        match name {
            field::ATTACH_TO_SUBPROCESS => self.draft.attach_to_subprocess = flag()?,
            field::GEVENT => self.draft.gevent = flag()?,
            field::PYQT => {
                self.draft.pyqt = flag()?;
                self.list = None;
            }
            field::PYQT_BACKEND => {
                let backend = value
                    .as_str()
                    .and_then(QtBackend::parse)
                    .ok_or("`pyqt-backend` takes `auto`, `pyqt4`, `pyqt5`, `pyqt6`, `pyside`, `pyside2` or `pyside6`")?;
                if !backend.debugpy_supports() {
                    return Err(format!("debugpy does not support {}", backend.name()));
                }
                self.choose(backend, window, cx);
            }
            field::TIMEOUT => {
                let text = match value {
                    Json::String(s) => s.clone(),
                    Json::Number(n) => n.to_string(),
                    _ => return Err("`evaluation-timeout` takes a number".into()),
                };
                self.put(&text, cx);
            }
            field::LIST => match value {
                Json::Null | Json::Bool(false) => self.close_list_in(window, cx),
                _ => self.open_list(window, cx),
            },
            "collect-types" | "drop-into-debugger" | "attach-filter" | "mode" => {
                return Err(format!("`{name}` is not supported yet"));
            }
            _ => return Err(format!("no field `{name}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for DebuggerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = div()
            .id("python-debugger-page")
            .debug_selector(|| "python-debugger-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col();
        if self.root.is_none() {
            return page.child(
                div()
                    .text_color(cx.theme().color(UiRole::TextMuted))
                    .child(NO_PROJECT),
            );
        }
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let danger = theme.color(UiRole::Danger);
        let timeout_error = self.timeout_ms(cx).is_err();
        let timeout = self.timeout.clone();
        let attach_filter = self.attach_filter.clone();
        let checks = div()
            .flex()
            .flex_col()
            // Each row is as wide as its content: the search spotlight's
            // outline hugs it.
            .items_start()
            .gap(px(CHECK_GAP_V))
            .child(self.check(
                field::ATTACH_TO_SUBPROCESS,
                self.draft.attach_to_subprocess,
                ATTACH_TO_SUBPROCESS,
                true,
                cx,
            ))
            .child(self.collect_types_row(cx))
            .child(self.check(field::GEVENT, self.draft.gevent, GEVENT, true, cx))
            .child(self.check("drop-into-debugger", false, DROP_INTO_DEBUGGER, false, cx))
            .child(self.qt_row(cx));
        // The label column is as wide as the widest label.
        let labels = div()
            .flex_none()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(FIELD_GAP))
            .child(
                div()
                    .h(px(FIELD_ROW))
                    .flex()
                    .items_center()
                    .child(Self::label(
                        ATTACH_FILTER,
                        Some(ATTACH_FILTER_BOLD),
                        false,
                        cx,
                    )),
            )
            .child(
                div()
                    .h(px(FIELD_ROW))
                    .flex()
                    .items_center()
                    .child(Self::label(TIMEOUT, None, true, cx)),
            );
        let fields = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(FIELD_GAP))
            .child(
                self.framed(
                    "debugger-attach-filter",
                    &attach_filter,
                    FieldKind::Disabled,
                    window,
                    cx,
                )
                .w_full(),
            )
            .child(
                self.framed(
                    "debugger-timeout",
                    &timeout,
                    FieldKind::Spinner {
                        error: timeout_error,
                    },
                    window,
                    cx,
                )
                .w(px(SPINNER_WIDTH)),
            );
        let overlay = self
            .list
            .as_ref()
            .map(|(menu, _)| ui::menu::overlay(menu, self.anchor.get().bottom_left(), window));
        page.gap(px(BLOCK_GAP))
            .pt(px(PAGE_TOP))
            .child(checks)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .gap(px(LABEL_GAP))
                    .child(labels)
                    .child(fields),
            )
            .child(
                div()
                    .mt(px(MODE_TOP))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(HINT_GAP))
                    .child(self.mode_row(cx))
                    .child(settings_ui::search::spotlight(
                        DEBUGPY_DESCRIPTION,
                        div().text_color(muted).child(DEBUGPY_DESCRIPTION),
                        cx,
                    )),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(danger).child(e)),
            )
            .children(overlay)
    }
}
