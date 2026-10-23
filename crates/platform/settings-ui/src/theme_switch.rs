//! The theme and the editor colour scheme of the running application (the
//! reference's `LafManager` with `QuickChangeLookAndFeel`): the catalog of
//! themes (built-in and imported), the theme the windows use, switching it
//! (stored at once; Cancel in the Settings window does not undo it), the
//! rows of the Theme popup with the preview while its selection moves, and
//! import.
//!
//! The theme in use is `[appearance] theme` of the settings, else the
//! default; `PYOXIDE_THEME` pins one for the run until a theme is chosen.
//! The editor colour scheme is the theme's own unless `[appearance]
//! editor-scheme` names another; choosing a theme goes back to its own.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Action, App, DismissEvent, Entity, EntityId, Global, PathPromptOptions, Subscription, Task,
};
use serde_json::{Value as Json, json};
use settings::AppearanceSettings;
use themes::import::ImportReport;
use themes::{Catalog, CatalogEntry, ThemeKind};
use ui::menu::{ContextMenu, HighlightEvent, MenuItem};
use ui::notification::{Notification, NotificationKind};
use ui::theme::ThemeExt;

use crate::commands::{ImportTheme, SwitchTheme};
use crate::store;

/// How long the selection rests on a theme before the IDE previews it
/// (`QuickChangeLookAndFeelService`: 500 ms).
pub const PREVIEW_DELAY: Duration = Duration::from_millis(500);
/// The Theme popup's title and its row in the Switch popup
/// (`action.ChangeLaf.text`).
pub const THEME: &str = "Theme";
/// The gear menu's row: the gear adds an ellipsis to its rows
/// (`SettingsEntryPointGroup`).
pub const THEME_IN_GEAR: &str = "Theme\u{2026}";
/// The quick switcher's title (`popup.title.switch`).
pub const SWITCH: &str = "Switch";
/// The last row of a theme list, in place of the reference's "Get More
/// Themes…" (`link.get.more.themes`).
pub const IMPORT_THEME: &str = "Import Theme\u{2026}";

/// Shows a balloon in a main window.
pub type Notify = Rc<dyn Fn(Notification, &mut App)>;

/// What the windows show.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    theme: String,
    scheme: Option<String>,
}

/// An open Theme popup: the preview waiting for the selection to rest.
struct Preview {
    menu: EntityId,
    pending: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

struct State {
    catalog: Catalog,
    /// The folder of imported themes.
    dir: Option<PathBuf>,
    /// `PYOXIDE_THEME`: the theme of this run until one is chosen.
    pinned: Option<String>,
    shown: Option<Shown>,
    preview: Option<Preview>,
    notify: Option<Notify>,
}

impl Global for State {}

fn state(cx: &mut App) -> &mut State {
    if !cx.has_global::<State>() {
        let shown = cx.try_global::<ui::theme::Theme>().map(|t| Shown {
            theme: t.id().to_owned(),
            scheme: None,
        });
        cx.set_global(State {
            catalog: Catalog::builtin(),
            dir: None,
            pinned: None,
            shown,
            preview: None,
            notify: None,
        });
    }
    cx.global_mut::<State>()
}

/// Registers the `[appearance]` section, the commands that need no window
/// (switching by id, import) and follows the stored settings.
pub fn init(cx: &mut App) {
    store::register::<AppearanceSettings>(cx);
    state(cx);
    cx.on_action(|a: &SwitchTheme, cx| {
        if let Some(id) = &a.id
            && let Err(e) = switch(id, cx)
        {
            eprintln!("settings.switch-theme: {e}");
        }
    });
    cx.on_action(|a: &ImportTheme, cx| match &a.path {
        Some(path) => import(PathBuf::from(path), cx),
        None => choose_and_import(cx),
    });
    store::observe(cx, <AppearanceSettings as settings::Section>::KEY, apply);
}

/// Reads the imported themes of `dir` (start-up, before the first window)
/// and shows the stored theme, or `pinned` (`PYOXIDE_THEME`: `light`,
/// `dark` or a theme id) for this run.
pub fn load(cx: &mut App, dir: Option<PathBuf>, pinned: Option<String>) {
    let (catalog, problems) = match &dir {
        Some(dir) => Catalog::load(dir),
        None => (Catalog::builtin(), Vec::new()),
    };
    for problem in problems {
        eprintln!("themes: {problem}");
    }
    let pinned = pinned.map(|p| match p.as_str() {
        "light" => themes::builtin::default_id_for(ThemeKind::Light).to_owned(),
        "dark" => themes::builtin::default_id_for(ThemeKind::Dark).to_owned(),
        _ => p,
    });
    let s = state(cx);
    s.catalog = catalog;
    s.dir = dir;
    s.pinned = pinned;
    apply(cx);
}

/// Where balloons about imports go (the workbench's main windows).
pub fn set_notify(cx: &mut App, notify: Notify) {
    state(cx).notify = Some(notify);
}

/// The themes the IDE offers.
pub fn catalog(cx: &App) -> &Catalog {
    static BUILTIN: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    match cx.try_global::<State>() {
        Some(s) => &s.catalog,
        None => BUILTIN.get_or_init(Catalog::builtin),
    }
}

fn default_theme() -> String {
    themes::builtin::default_id_for(ThemeKind::Dark).to_owned()
}

/// The theme in use: pinned for the run, else stored, else the default;
/// one the catalog does not know gives the default.
pub fn current_theme(cx: &App) -> String {
    let Some(s) = cx.try_global::<State>() else {
        return default_theme();
    };
    let stored: AppearanceSettings = store::get(cx);
    s.pinned
        .clone()
        .or(stored.theme)
        .filter(|id| s.catalog.is_theme(id))
        .unwrap_or_else(default_theme)
}

/// The editor colour scheme in use: the stored one for the stored theme,
/// else the theme's own.
pub fn current_scheme(cx: &App) -> String {
    let theme = current_theme(cx);
    let Some(s) = cx.try_global::<State>() else {
        return theme;
    };
    if s.pinned.is_some() {
        return theme;
    }
    let stored: AppearanceSettings = store::get(cx);
    stored
        .editor_scheme
        .filter(|id| s.catalog.get(id).is_some())
        .unwrap_or(theme)
}

/// Shows `theme` with `scheme` in every window unless they show it.
fn show(theme: &str, scheme: Option<&str>, cx: &mut App) {
    let wanted = Shown {
        theme: theme.to_owned(),
        scheme: scheme.filter(|s| *s != theme).map(str::to_owned),
    };
    if state(cx).shown.as_ref() == Some(&wanted) {
        return;
    }
    match state(cx)
        .catalog
        .compose(&wanted.theme, wanted.scheme.as_deref())
    {
        Ok(theme) => {
            ui::theme::set(cx, theme);
            state(cx).shown = Some(wanted);
        }
        Err(e) => eprintln!("themes: {e}"),
    }
}

/// Shows the theme and scheme in use (after a change of the settings, and
/// to end a preview).
pub fn apply(cx: &mut App) {
    let theme = current_theme(cx);
    let scheme = current_scheme(cx);
    show(&theme, Some(&scheme), cx);
}

/// Makes `id` the theme at once and stores it; the editor colour scheme
/// goes back to the theme's own (`switchLafAndUpdateUI`).
pub fn switch(id: &str, cx: &mut App) -> Result<(), String> {
    if !state(cx).catalog.is_theme(id) {
        return Err(format!("unknown theme `{id}`"));
    }
    state(cx).pinned = None;
    store::set(
        cx,
        &AppearanceSettings {
            theme: Some(id.to_owned()),
            editor_scheme: None,
        },
    );
    apply(cx);
    Ok(())
}

/// Stores `id` as the editor colour scheme of the theme in use (Apply on
/// the Appearance page).
pub fn set_scheme(id: &str, cx: &mut App) -> Result<(), String> {
    if state(cx).catalog.get(id).is_none() {
        return Err(format!("unknown editor color scheme `{id}`"));
    }
    let theme = current_theme(cx);
    state(cx).pinned = None;
    store::set(
        cx,
        &AppearanceSettings {
            editor_scheme: (id != theme).then(|| id.to_owned()),
            theme: Some(theme),
        },
    );
    apply(cx);
    Ok(())
}

/// Shows `id` with its own scheme without storing it.
pub fn preview(id: &str, cx: &mut App) {
    if state(cx).catalog.is_theme(id) {
        show(id, None, cx);
    }
}

/// The theme the windows show (a preview, or the theme in use).
pub fn shown_theme(cx: &App) -> String {
    cx.try_global::<State>()
        .and_then(|s| s.shown.as_ref().map(|s| s.theme.clone()))
        .unwrap_or_else(|| cx.theme().id().to_owned())
}

/// The rows of the Theme popup (`QuickChangeLookAndFeel.fillActions`):
/// the themes of each group, a line between groups, then "Import Theme…".
/// The theme in use has no mark of its own: the popup opens on it
/// (`ide.instant.theme.switch` is on).
pub fn theme_items(cx: &App) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let mut group = None;
    for entry in catalog(cx).themes() {
        if group.is_some_and(|g| g != entry.group) {
            items.push(MenuItem::separator());
        }
        group = Some(entry.group);
        items.push(MenuItem::action(
            entry.name.clone(),
            SwitchTheme {
                id: Some(entry.id.clone()),
            },
        ));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action(IMPORT_THEME, ImportTheme { path: None }));
    items
}

/// The row of the theme in use among [`theme_items`].
pub fn current_row(items: &[MenuItem], cx: &App) -> Option<usize> {
    let current = current_theme(cx);
    items.iter().position(|item| match item {
        MenuItem::Action {
            action: Some(action),
            ..
        } => action
            .as_any()
            .downcast_ref::<SwitchTheme>()
            .is_some_and(|a| a.id.as_deref() == Some(current.as_str())),
        _ => false,
    })
}

/// Previews the themes of the Theme popup `menu` as its selection moves:
/// half a second after a theme row is selected the IDE shows it; the
/// popup closing without a theme chosen brings back the theme in use
/// (`QuickChangeLookAndFeelService.preparePopup`). A theme chosen in the
/// popup runs after it closed. Rows that are not themes change nothing.
pub fn watch(menu: &Entity<ContextMenu>, cx: &mut App) {
    let id = menu.entity_id();
    let subscriptions = vec![
        cx.subscribe(menu, move |_, e: &HighlightEvent, cx| {
            highlighted(id, e.0.as_deref(), cx)
        }),
        cx.subscribe(menu, move |_, _: &DismissEvent, cx| end_preview(id, cx)),
    ];
    // A popup opened before ends its preview.
    if let Some(earlier) = state(cx).preview.take() {
        cx.defer(move |_| drop(earlier));
    }
    state(cx).preview = Some(Preview {
        menu: id,
        pending: None,
        _subscriptions: subscriptions,
    });
}

fn highlighted(menu: EntityId, action: Option<&dyn Action>, cx: &mut App) {
    let Some(SwitchTheme { id: Some(id) }) =
        action.and_then(|a| a.as_any().downcast_ref::<SwitchTheme>())
    else {
        return;
    };
    let id = id.clone();
    if !state(cx).preview.as_ref().is_some_and(|p| p.menu == menu) {
        return;
    }
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(PREVIEW_DELAY).await;
        let _ = cx.update(|cx| preview(&id, cx));
    });
    if let Some(open) = state(cx).preview.as_mut() {
        open.pending = Some(task);
    }
}

fn end_preview(menu: EntityId, cx: &mut App) {
    if !state(cx).preview.as_ref().is_some_and(|p| p.menu == menu) {
        return;
    }
    let Some(preview) = state(cx).preview.take() else {
        return;
    };
    apply(cx);
    // Its subscriptions are the caller's; they go after it.
    cx.defer(move |_| drop(preview));
}

/// Shows the file chooser and imports what is chosen.
pub fn choose_and_import(cx: &mut App) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: true,
        multiple: false,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = chosen.await
            && let Some(path) = paths.into_iter().next()
        {
            let _ = cx.update(|cx| import(path, cx));
        }
    })
    .detach();
}

/// Imports a theme file, scheme, plugin archive or folder in the
/// background into the themes folder; the first UI theme becomes the
/// theme, and a balloon tells what happened.
pub fn import(path: PathBuf, cx: &mut App) {
    let Some(dir) = state(cx).dir.clone() else {
        report(
            Notification::new(
                NotificationKind::Error,
                "Theme import failed",
                "There is no configuration folder to keep imported themes in.",
            ),
            cx,
        );
        return;
    };
    let work = cx
        .background_executor()
        .spawn(async move { themes::catalog::import_into(&path, &dir) });
    cx.spawn(async move |cx| {
        let result = work.await;
        let _ = cx.update(|cx| imported(result, cx));
    })
    .detach();
}

/// Adds imported theme files to the catalog without switching or a
/// balloon; returns the ids of the themes and of the schemes added, and
/// logs what each import could not map.
pub fn add_imported(
    files: Vec<themes::catalog::ImportedFile>,
    cx: &mut App,
) -> (Vec<String>, Vec<String>) {
    let mut themes = Vec::new();
    let mut schemes = Vec::new();
    let mut reports = Vec::new();
    for file in files {
        let scheme_only = file.file.theme.scheme_only;
        match state(cx).catalog.add(file.file, Some(file.path)) {
            Ok(id) if scheme_only => schemes.push(id),
            Ok(id) => themes.push(id),
            Err(e) => eprintln!("themes: {e}"),
        }
        reports.push(file.report);
    }
    log_reports(&reports);
    (themes, schemes)
}

/// Makes `theme` the theme with `scheme` as its editor colour scheme
/// (`None`: its own) and stores both.
pub fn set_theme_and_scheme(theme: &str, scheme: Option<&str>, cx: &mut App) -> Result<(), String> {
    if !state(cx).catalog.is_theme(theme) {
        return Err(format!("unknown theme `{theme}`"));
    }
    if let Some(scheme) = scheme
        && state(cx).catalog.get(scheme).is_none()
    {
        return Err(format!("unknown editor color scheme `{scheme}`"));
    }
    state(cx).pinned = None;
    store::set(
        cx,
        &AppearanceSettings {
            theme: Some(theme.to_owned()),
            editor_scheme: scheme.filter(|s| *s != theme).map(str::to_owned),
        },
    );
    apply(cx);
    Ok(())
}

fn imported(
    result: Result<Vec<themes::catalog::ImportedFile>, themes::import::ImportError>,
    cx: &mut App,
) {
    let files = match result {
        Ok(files) => files,
        Err(e) => {
            report(
                Notification::new(
                    NotificationKind::Error,
                    "Theme import failed",
                    e.to_string(),
                ),
                cx,
            );
            return;
        }
    };
    let mut themes = Vec::new();
    let mut schemes = Vec::new();
    let mut reports = Vec::new();
    for file in files {
        let scheme_only = file.file.theme.scheme_only;
        let name = file.file.theme.name.clone();
        match state(cx).catalog.add(file.file, Some(file.path)) {
            Ok(_) if scheme_only => schemes.push(name),
            Ok(id) => themes.push((id, name)),
            Err(e) => eprintln!("themes: {e}"),
        }
        reports.push(file.report);
    }
    if let Some((id, _)) = themes.first()
        && let Err(e) = switch(id, cx)
    {
        eprintln!("themes: {e}");
    }
    log_reports(&reports);
    report(import_notification(&themes, &schemes), cx);
    cx.refresh_windows();
}

/// The balloon of a finished import: what came in, in a sentence or two
/// (the reference says "Color scheme added" for a scheme). What could not
/// be mapped stays in the import report, not in the balloon or the log.
fn import_notification(themes: &[(String, String)], schemes: &[String]) -> Notification {
    let (title, text) = match (themes, schemes) {
        ([], []) => {
            return Notification::new(
                NotificationKind::Warning,
                "Nothing imported",
                "No UI theme or editor color scheme could be imported.",
            );
        }
        ([(_, name)], []) => (
            "Theme imported".to_owned(),
            format!("{name} is now the theme."),
        ),
        ([], [name]) => (
            "Color scheme added".to_owned(),
            format!("{name} is offered as an editor color scheme."),
        ),
        (themes, schemes) => {
            let mut parts = Vec::new();
            if !themes.is_empty() {
                parts.push(plural(themes.len(), "theme"));
            }
            if !schemes.is_empty() {
                parts.push(plural(schemes.len(), "editor color scheme"));
            }
            let mut text = format!("Imported {}.", parts.join(" and "));
            if let Some((_, name)) = themes.first() {
                text.push_str(&format!(" {name} is now the theme."));
            }
            ("Themes imported".to_owned(), text)
        }
    };
    Notification::new(NotificationKind::Information, title, text)
}

/// Writes one short line per imported file to the log (stderr); what could
/// not be mapped stays in the [`ImportReport`] (`ImportReport::render`).
fn log_reports(reports: &[ImportReport]) {
    for r in reports {
        let summary = r.summary();
        eprintln!(
            "themes: imported {}: {} mapped, {} not mapped",
            r.name, summary.mapped, summary.unmapped_keys,
        );
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn report(notification: Notification, cx: &mut App) {
    match state(cx).notify.clone() {
        Some(notify) => notify(notification, cx),
        None => eprintln!("{}: {}", notification.title, notification.content),
    }
}

fn entry_json(e: &CatalogEntry) -> Json {
    json!({
        "id": e.id,
        "name": e.name,
        "group": e.group.name(),
        "dark": e.kind == ThemeKind::Dark,
        "scheme_only": e.scheme_only,
    })
}

/// The themes and the theme in use for `settings.state`.
pub fn state_json(cx: &App) -> Json {
    let Some(s) = cx.try_global::<State>() else {
        return Json::Null;
    };
    json!({
        "theme": current_theme(cx),
        "editor_scheme": current_scheme(cx),
        "shown": shown_theme(cx),
        "pinned": s.pinned,
        "previewing": s.preview.is_some(),
        "themes": s.catalog.themes().map(entry_json).collect::<Vec<_>>(),
        "schemes": s.catalog.schemes().filter(|e| e.scheme_only).map(entry_json).collect::<Vec<_>>(),
    })
}
