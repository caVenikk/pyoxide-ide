//! Tools > Terminal. Project options and the application shell preferences
//! are applied with the font draft. Unchanged font fields stay inherited.

use std::path::PathBuf;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, IntoElement, Render,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings::{TerminalFontSettings, TerminalSettings};
use settings_ui::pages::family_combo::{ComboEvent, ComboSpec, FamilyCombo};
use settings_ui::{PageContext, SettingsPage};

use crate::terminal::project_options::{self, Options as ProjectOptions};

pub const ID: &str = "terminal";
pub const TITLE: &str = "Terminal";
pub const SEARCH_TERMS: &[&str] = &[
    PROJECT_SETTINGS,
    START_DIRECTORY,
    ENVIRONMENT,
    APPLICATION_SETTINGS,
    SHELL_PATH,
    CLOSE,
    OVERRIDE,
    INTEGRATION,
    META,
    COLORS,
    FONT_SETTINGS,
    FONT,
    FALLBACK,
    SIZE,
    LINE_HEIGHT,
    COLUMN_WIDTH,
];

/// The reference's labels (`TerminalBundle`).
const PROJECT_SETTINGS: &str = "Project Settings";
const APPLICATION_SETTINGS: &str = "Application Settings";
const START_DIRECTORY: &str = "Start directory:";
const ENVIRONMENT: &str = "Environment variables:";
const SHELL_PATH: &str = "Shell path:";
const CLOSE: &str = "Close session when it ends";
const OVERRIDE: &str = "Override IDE shortcuts";
const INTEGRATION: &str = "Shell integration";
const META: &str = "Use Option as Meta key";
const COLORS: &str = "Configure colors…";
const FONT_SETTINGS: &str = "Font Settings";
const FONT: &str = "Font:";
const FALLBACK: &str = "Fallback:";
const SIZE: &str = "Size:";
const LINE_HEIGHT: &str = "Line height:";
const COLUMN_WIDTH: &str = "Column width:";
/// The fallback row that stands for the platform's own fallback (the
/// reference's logical font `Monospaced`).
const MONOSPACED: &str = "Monospaced";

/// Kotlin UI DSL spacing: a group's indent, the gap between rows, the gap
/// between a label and its field, between the cells of a row, and the width
/// of a four-column field.
const GROUP_INDENT: f32 = 20.0;
const ROW_GAP: f32 = 6.0;
const ROW_HEIGHT: f32 = 24.0;
const LABEL_GAP: f32 = 6.0;
const CELL_GAP: f32 = 17.0;
const NUMBER_WIDTH: f32 = 52.0;
const COMBO_WIDTH: f32 = 220.0;

/// The combo boxes: monospaced families only, without groups
/// (`isMonospacedOnly`); the fallback's first row is `Monospaced`.
const FAMILY_COMBO: ComboSpec = ComboSpec {
    prefix: "terminal-font",
    name: "family",
    width: COMBO_WIDTH,
    list_width: COMBO_WIDTH,
    monospaced_only: true,
    groups: false,
    names_only: true,
    none: None,
    warn_missing: true,
};
const FALLBACK_COMBO: ComboSpec = ComboSpec {
    name: "fallback",
    none: Some(MONOSPACED),
    warn_missing: false,
    ..FAMILY_COMBO
};

/// The keys of `settings.edit`.
mod field {
    pub const FAMILY: &str = "family";
    pub const FALLBACK: &str = "fallback";
    pub const SIZE: &str = "size";
    pub const LINE_HEIGHT: &str = "line-height";
    pub const COLUMN_WIDTH: &str = "column-width";
    pub const LIST: &str = "list";
    pub const CHOOSE: &str = "choose";
}

/// The font the terminal has when nothing is chosen: the editor font (or the
/// theme's terminal font), as the terminal view lays it out.
#[derive(Debug, Clone, PartialEq)]
struct Inherited {
    family: String,
    size: f32,
}

impl Inherited {
    fn of(window: &mut Window, cx: &App) -> Self {
        let metrics = terminal_ui::cell_metrics(window, cx);
        Self {
            family: metrics.font.family.to_string(),
            size: TerminalFontSettings::clamp_size(f32::from(metrics.font_size)),
        }
    }
}

/// What the page's fields hold.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    family: String,
    fallback: Option<String>,
    size: f32,
    line_spacing: f32,
    column_spacing: f32,
}

impl Shown {
    /// What the settings give: a field not chosen is the inherited font or 1.
    fn of(settings: &TerminalFontSettings, inherited: &Inherited) -> Self {
        Self {
            family: settings
                .family
                .clone()
                .unwrap_or_else(|| inherited.family.clone()),
            fallback: settings.fallback_family().map(str::to_owned),
            size: settings
                .size
                .map(TerminalFontSettings::clamp_size)
                .unwrap_or(inherited.size),
            line_spacing: settings
                .line_spacing
                .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                    TerminalFontSettings::clamp_spacing(s)
                }),
            column_spacing: settings
                .column_spacing
                .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                    TerminalFontSettings::clamp_spacing(s)
                }),
        }
    }

    /// The settings to store: a field the person did not change keeps what
    /// is stored (still inherited when it was), a changed one is chosen.
    fn settings(
        &self,
        stored: &TerminalFontSettings,
        inherited: &Inherited,
    ) -> TerminalFontSettings {
        let before = Self::of(stored, inherited);
        fn pick<T: PartialEq>(now: T, before: T, stored: Option<T>) -> Option<T> {
            if now == before { stored } else { Some(now) }
        }
        TerminalFontSettings {
            family: pick(self.family.clone(), before.family, stored.family.clone()),
            size: pick(self.size, before.size, stored.size),
            line_spacing: pick(self.line_spacing, before.line_spacing, stored.line_spacing),
            column_spacing: pick(
                self.column_spacing,
                before.column_spacing,
                stored.column_spacing,
            ),
            fallback: if self.fallback == before.fallback {
                stored.fallback.clone()
            } else {
                self.fallback.clone()
            },
        }
    }
}

/// The page: the combo boxes and the number fields as typed.
pub struct TerminalPage {
    inherited: Inherited,
    root: Option<PathBuf>,
    project: Option<ProjectOptions>,
    toggle: Option<bool>,
    product_toggle: Option<crate::terminal::ProjectToggle>,
    error: Option<String>,
    revision: u64,
    app: TerminalSettings,
    shell: Entity<Editor>,
    directory: Entity<Editor>,
    environment: Entity<Editor>,
    family: Entity<FamilyCombo>,
    fallback: Entity<FamilyCombo>,
    size: Entity<Editor>,
    line_spacing: Entity<Editor>,
    column_spacing: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPage {
    pub fn new(context: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inherited = Inherited::of(window, cx);
        let shown = Shown::of(&settings_ui::store::get(cx), &inherited);
        let focus = cx.focus_handle();
        let combo = |spec: ComboSpec, value: Option<String>, cx: &mut Context<Self>| {
            let focus = focus.clone();
            cx.new(|cx| FamilyCombo::new(spec, value, focus, cx))
        };
        let family = combo(FAMILY_COMBO, Some(shown.family.clone()), cx);
        let fallback = combo(FALLBACK_COMBO, shown.fallback.clone(), cx);
        let number = |text: String, window: &mut Window, cx: &mut Context<Self>| {
            editor::field::new(&text, None, true, window, cx)
        };
        let size = number(TerminalFontSettings::format_size(shown.size), window, cx);
        let line_spacing = number(
            TerminalFontSettings::format_spacing(shown.line_spacing),
            window,
            cx,
        );
        let column_spacing = number(
            TerminalFontSettings::format_spacing(shown.column_spacing),
            window,
            cx,
        );
        let edited = |_: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
            if matches!(e, EditorEvent::Edited(_)) {
                cx.notify();
            }
        };
        let chosen = |_: &mut Self,
                      _: Entity<FamilyCombo>,
                      _: &ComboEvent,
                      cx: &mut Context<Self>| { cx.notify() };
        let app: TerminalSettings = settings_ui::store::get(cx);
        let shell = editor::field::new(
            &app.shell_path,
            Some("System default shell".into()),
            true,
            window,
            cx,
        );
        let directory = editor::field::new(
            "",
            context
                .project
                .as_ref()
                .map(|root| root.display().to_string().into()),
            true,
            window,
            cx,
        );
        let environment =
            editor::field::new("", Some("NAME=value;NAME2=value".into()), true, window, cx);
        if let Some(root) = &context.project {
            let task = project_options::read(root, cx);
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(options) if options.requires_isolated_env => {
                            this.error = Some(terminal::SpawnError::IsolatedEnvironment.to_string())
                        }
                        Ok(options) => this.show_project(options, cx),
                        Err(e) => this.error = Some(e),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let subscriptions = vec![
            cx.subscribe(&shell, edited),
            cx.subscribe(&directory, edited),
            cx.subscribe(&environment, edited),
            cx.subscribe(&size, edited),
            cx.subscribe(&line_spacing, edited),
            cx.subscribe(&column_spacing, edited),
            cx.subscribe(&family, chosen),
            cx.subscribe(&fallback, chosen),
        ];
        Self {
            root: context.project.clone(),
            project: None,
            toggle: None,
            product_toggle: crate::terminal::project_toggle(cx),
            error: None,
            revision: 0,
            shell,
            directory,
            environment,
            app,
            inherited,
            family,
            fallback,
            size,
            line_spacing,
            column_spacing,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn text(field: &Entity<Editor>, cx: &App) -> String {
        field.read(cx).text().trim().to_owned()
    }

    /// The fields as they are now; a number that is no number is the
    /// default, and out of range it is kept in range.
    fn draft(&self, cx: &App) -> Shown {
        Shown {
            family: self
                .family
                .read(cx)
                .value()
                .unwrap_or(&self.inherited.family)
                .to_owned(),
            fallback: self.fallback.read(cx).value().map(str::to_owned),
            size: TerminalFontSettings::parse_size(&Self::text(&self.size, cx)),
            line_spacing: TerminalFontSettings::parse_spacing(&Self::text(&self.line_spacing, cx)),
            column_spacing: TerminalFontSettings::parse_spacing(&Self::text(
                &self.column_spacing,
                cx,
            )),
        }
    }

    fn stored(&self, cx: &App) -> TerminalFontSettings {
        settings_ui::store::get(cx)
    }

    /// Puts `shown` in the fields.
    fn show(&mut self, shown: &Shown, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.family
            .update(cx, |c, cx| c.set_value(Some(shown.family.clone()), cx));
        self.fallback
            .update(cx, |c, cx| c.set_value(shown.fallback.clone(), cx));
        let size = TerminalFontSettings::format_size(shown.size);
        let line = TerminalFontSettings::format_spacing(shown.line_spacing);
        let column = TerminalFontSettings::format_spacing(shown.column_spacing);
        self.size.update(cx, |e, cx| e.set_text_selected(&size, cx));
        self.line_spacing
            .update(cx, |e, cx| e.set_text_selected(&line, cx));
        self.column_spacing
            .update(cx, |e, cx| e.set_text_selected(&column, cx));
        let mut window = window;
        for combo in [&self.family, &self.fallback] {
            combo.update(cx, |c, cx| c.close(window.as_deref_mut(), cx));
        }
        cx.notify();
    }

    fn show_project(&mut self, options: ProjectOptions, cx: &mut Context<Self>) {
        self.directory
            .update(cx, |e, cx| e.set_text_selected(&options.directory, cx));
        self.environment.update(cx, |e, cx| {
            e.set_text_selected(&run_config::form::env_text(&options.env), cx)
        });
        self.toggle = options.toggle;
        self.project = Some(options);
    }

    fn project_draft(&self, cx: &App) -> Option<ProjectOptions> {
        let stored = self.project.as_ref()?;
        let text = self.environment.read(cx).text();
        let env = if text == run_config::form::env_text(&stored.env) {
            stored.env.clone()
        } else {
            run_config::form::parse_env(&text, &[])
        };
        Some(ProjectOptions {
            requires_isolated_env: false,
            toggle: self.toggle,
            directory: Self::text(&self.directory, cx),
            env,
        })
    }

    fn app_draft(&self, cx: &App) -> TerminalSettings {
        TerminalSettings {
            shell_path: self.shell.read(cx).text(),
            ..self.app.clone()
        }
    }

    fn option(&mut self, key: &str) -> Option<&mut bool> {
        match key {
            "shell-integration" => Some(&mut self.app.shell_integration),
            "close-session-on-exit" => Some(&mut self.app.close_session_on_exit),
            "override-ide-shortcuts" => Some(&mut self.app.override_ide_shortcuts),
            "option-as-meta" if cfg!(target_os = "macos") => Some(&mut self.app.option_as_meta),
            _ if self.product_toggle.is_some_and(|t| t.key == key) => self.toggle.as_mut(),
            _ => None,
        }
    }

    fn check(
        &self,
        key: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        ui::form::check_row(
            gpui::ElementId::Name(format!("terminal-row:{key}").into()),
            gpui::ElementId::Name(format!("terminal-checkbox:{key}").into()),
            checked,
            settings_ui::search::label(label, cx),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(value) = this.option(key) {
                *value = !*value;
                cx.notify();
            }
        }))
        .into_any_element()
    }

    fn input(label: &'static str, field: &Entity<Editor>, window: &Window, cx: &App) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(LABEL_GAP))
            .child(settings_ui::search::label(label, cx))
            .child(
                div().w(px(320.)).child(
                    editor::field::frame(field, true, false, window, cx)
                        .h(px(ROW_HEIGHT))
                        .px(px(6.)),
                ),
            )
            .into_any_element()
    }

    fn number_field(&self, name: &'static str, window: &Window, cx: &App) -> AnyElement {
        let field = match name {
            field::SIZE => &self.size,
            field::LINE_HEIGHT => &self.line_spacing,
            _ => &self.column_spacing,
        };
        div()
            .id(name)
            .debug_selector(move || format!("terminal-font:{name}"))
            .flex_none()
            .w(px(NUMBER_WIDTH))
            .child(
                editor::field::frame(field, true, false, window, cx)
                    .h(px(ROW_HEIGHT))
                    .pl(px(6.0))
                    .pr(px(4.0)),
            )
            .into_any_element()
    }
}

impl SettingsPage for TerminalPage {
    fn is_modified(&self, cx: &App) -> bool {
        let stored = self.stored(cx);
        self.app_draft(cx) != settings_ui::store::get::<TerminalSettings>(cx)
            || self.project_draft(cx) != self.project
            || self.draft(cx).settings(&stored, &self.inherited) != stored
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let stored = self.stored(cx);
        if let (Some(root), Some(stored), Some(options)) = (
            self.root.clone(),
            self.project.clone(),
            self.project_draft(cx),
        ) && stored != options
        {
            if options.env.iter().any(|v| v.value.is_none()) {
                return Err(
                    "Enter environment variables as NAME=value separated by semicolons.".to_owned(),
                );
            }
            let before = self.project.replace(options.clone());
            let write = project_options::save(root, stored, options, cx);
            self.revision += 1;
            let revision = self.revision;
            cx.spawn(async move |this, cx| {
                if let Err(error) = write.await {
                    let _ = cx.update(|cx| {
                        super::notify_main_window(
                            ui::notification::Notification::new(
                                ui::notification::NotificationKind::Error,
                                "Could not save terminal settings",
                                error.clone(),
                            ),
                            cx,
                        )
                    });
                    let _ = this.update(cx, |this, cx| {
                        if this.revision == revision {
                            this.project = before;
                            this.error = Some(error);
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
            self.error = None;
        }
        let app = self.app_draft(cx);
        settings_ui::store::set(cx, &app);
        let settings = self.draft(cx).settings(&stored, &self.inherited);
        settings_ui::store::set(cx, &settings);
        // The fields show the numbers kept in range.
        let shown = Shown::of(&settings, &self.inherited);
        self.show(&shown, Some(window), cx);
        Ok(())
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app = settings_ui::store::get(cx);
        self.shell
            .update(cx, |e, cx| e.set_text_selected(&self.app.shell_path, cx));
        if let Some(project) = self.project.clone() {
            self.show_project(project, cx);
        }
        self.inherited = Inherited::of(window, cx);
        let shown = Shown::of(&self.stored(cx), &self.inherited);
        self.show(&shown, Some(window), cx);
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let shown = self.draft(cx);
        let open = [
            (field::FAMILY, &self.family),
            (field::FALLBACK, &self.fallback),
        ]
        .into_iter()
        .find(|(_, c)| c.read(cx).is_open())
        .map(|(name, c)| (name, c.read(cx).state()));
        let mut state = json!({
            "shell-path": self.shell.read(cx).text(),
            "shell-integration": self.app.shell_integration,
            "close-session-on-exit": self.app.close_session_on_exit,
            "override-ide-shortcuts": self.app.override_ide_shortcuts,
            "option-as-meta": self.app.option_as_meta,
            "start-directory": self.project.as_ref().map(|_| Self::text(&self.directory,cx)),
            "environment": self.project.as_ref().map(|_| Self::text(&self.environment,cx)),
            "error": self.error,
            "family": shown.family,
            "fallback": shown.fallback.clone().unwrap_or_else(|| MONOSPACED.to_owned()),
            "size": Self::text(&self.size, cx),
            "line-height": Self::text(&self.line_spacing, cx),
            "column-width": Self::text(&self.column_spacing, cx),
            "inherited": {"family": self.inherited.family, "size": self.inherited.size},
            "list": open.as_ref().map(|(name, _)| name),
            "rows": open.as_ref().map(|(_, state)| state["rows"].clone()),
            "highlighted": open.as_ref().map(|(_, state)| state["highlighted"].clone()),
        });
        if let Some(t) = self.product_toggle {
            state[t.key] = json!(self.toggle);
        }
        state
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || match value {
            Json::String(s) => Ok(s.clone()),
            Json::Number(n) => Ok(n.to_string()),
            _ => Err(format!("`{name}` takes a string or a number")),
        };
        if let Some(option) = self.option(name) {
            *option = value.as_bool().ok_or("Expected a boolean")?;
            cx.notify();
            return Ok(());
        }
        match name {
            "shell-path" | "start-directory" | "environment" => {
                let field = match name {
                    "shell-path" => &self.shell,
                    _ if self.project.is_none() => return Err("No project settings loaded".into()),
                    "start-directory" => &self.directory,
                    _ => &self.environment,
                };
                let value = value.as_str().ok_or("Expected text")?;
                field.update(cx, |e, cx| e.set_text_selected(value, cx));
            }
            field::FAMILY => {
                let family = value
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("`family` takes a family name")?
                    .to_owned();
                self.family
                    .update(cx, |c, cx| c.set_value(Some(family), cx));
            }
            field::FALLBACK => {
                let fallback = match value {
                    Json::Null => None,
                    Json::String(s) if s.is_empty() || s == MONOSPACED => None,
                    Json::String(s) => Some(s.clone()),
                    _ => return Err("`fallback` takes a family name or null".into()),
                };
                self.fallback.update(cx, |c, cx| c.set_value(fallback, cx));
            }
            field::SIZE | field::LINE_HEIGHT | field::COLUMN_WIDTH => {
                let t = text()?;
                let target = match name {
                    field::SIZE => &self.size,
                    field::LINE_HEIGHT => &self.line_spacing,
                    _ => &self.column_spacing,
                };
                target.update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            // Opens (`"family"`, `"fallback"`) or closes (`null`) a list.
            field::LIST => {
                for combo in [&self.family, &self.fallback] {
                    combo.update(cx, |c, cx| c.close(Some(&mut *window), cx));
                }
                match value.as_str() {
                    Some(field::FAMILY) => self.family.update(cx, |c, cx| c.open(window, cx)),
                    Some(field::FALLBACK) => self.fallback.update(cx, |c, cx| c.open(window, cx)),
                    None => {}
                    Some(other) => return Err(format!("no list `{other}`")),
                }
            }
            // Chooses a row of the open list by its label.
            field::CHOOSE => {
                let label = value.as_str().ok_or("`choose` takes a row's label")?;
                let combo = [&self.family, &self.fallback]
                    .into_iter()
                    .find(|c| c.read(cx).is_open())
                    .ok_or_else(|| format!("no row `{label}` in an open list"))?
                    .clone();
                combo.update(cx, |c, cx| c.choose_label(label, window, cx))?;
            }
            _ => return Err(format!("no field `{name}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for TerminalPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str, cx: &App| settings_ui::search::label(text, cx);
        let font_row = div()
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .child(label(FONT, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.family.clone())
            .child(div().w(px(CELL_GAP)))
            .child(label(FALLBACK, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.fallback.clone());
        let numbers_row = div()
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .child(label(SIZE, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::SIZE, window, cx))
            .child(div().w(px(CELL_GAP)))
            .child(label(LINE_HEIGHT, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::LINE_HEIGHT, window, cx))
            .child(div().w(px(CELL_GAP)))
            .child(label(COLUMN_WIDTH, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::COLUMN_WIDTH, window, cx));
        div()
            .id("terminal-page")
            .debug_selector(|| "terminal-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(ui::form::group_header(PROJECT_SETTINGS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .when(self.project.is_some(), |group| {
                        group
                            .child(Self::input(START_DIRECTORY, &self.directory, window, cx))
                            .child(Self::input(ENVIRONMENT, &self.environment, window, cx))
                    })
                    .when_some(self.product_toggle.zip(self.toggle), |group, (t, value)| {
                        group.child(self.check(t.key, t.label, value, cx))
                    })
                    .when(self.root.is_none(), |group| {
                        group.child("Open a project to configure its terminal.")
                    })
                    .when_some(self.error.clone(), |group, error| group.child(error)),
            )
            .child(ui::form::group_header(FONT_SETTINGS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(font_row)
                    .child(numbers_row)
                    .child(
                        ui::link::link("terminal-configure-colors", COLORS, cx).on_click(
                            |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(settings_ui::commands::SelectPage {
                                        page: settings_ui::pages::appearance::ID.into(),
                                    }),
                                    cx,
                                )
                            },
                        ),
                    ),
            )
            .child(ui::form::group_header(APPLICATION_SETTINGS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(Self::input(SHELL_PATH, &self.shell, window, cx))
                    .child(self.check(
                        "close-session-on-exit",
                        CLOSE,
                        self.app.close_session_on_exit,
                        cx,
                    ))
                    .child(self.check(
                        "override-ide-shortcuts",
                        OVERRIDE,
                        self.app.override_ide_shortcuts,
                        cx,
                    ))
                    .child(self.check(
                        "shell-integration",
                        INTEGRATION,
                        self.app.shell_integration,
                        cx,
                    ))
                    .when(cfg!(target_os = "macos"), |group| {
                        group.child(self.check("option-as-meta", META, self.app.option_as_meta, cx))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inherited() -> Inherited {
        Inherited {
            family: "JetBrains Mono".into(),
            size: 13.0,
        }
    }

    #[test]
    fn a_field_left_alone_stays_inherited() {
        let none = TerminalFontSettings::default();
        let shown = Shown::of(&none, &inherited());
        assert_eq!(shown.family, "JetBrains Mono");
        assert_eq!(shown.size, 13.0);
        assert_eq!(shown.line_spacing, 1.0);
        assert_eq!(shown.fallback, None);
        // Nothing changed: nothing is stored.
        assert_eq!(shown.settings(&none, &inherited()), none);

        // Only the size changed: only the size is chosen.
        let chosen = Shown {
            size: 15.0,
            ..shown
        };
        assert_eq!(
            chosen.settings(&none, &inherited()),
            TerminalFontSettings {
                size: Some(15.0),
                ..none.clone()
            }
        );
    }

    #[test]
    fn a_chosen_field_stays_chosen_and_the_logical_fallback_is_none() {
        let stored = TerminalFontSettings {
            family: Some("Menlo".into()),
            fallback: Some("Monospaced".into()),
            column_spacing: Some(1.1),
            ..TerminalFontSettings::default()
        };
        let shown = Shown::of(&stored, &inherited());
        assert_eq!(shown.family, "Menlo");
        assert_eq!(shown.fallback, None);
        assert_eq!(shown.column_spacing, 1.1);
        // Applied as it is, the stored values are kept as they are.
        assert_eq!(shown.settings(&stored, &inherited()), stored);

        // Another fallback is chosen; the family goes back to the editor's.
        let changed = Shown {
            family: "JetBrains Mono".into(),
            fallback: Some("Symbols Nerd Font Mono".into()),
            ..shown
        };
        let settings = changed.settings(&stored, &inherited());
        assert_eq!(settings.family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(settings.fallback.as_deref(), Some("Symbols Nerd Font Mono"));
        assert_eq!(settings.column_spacing, Some(1.1));
    }
}
