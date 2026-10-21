//! The first-run window (ADR 0017, the reference's onboarding dialog of
//! `intellij.ide.startup.importSettings`): at the first start, before the
//! main window, it offers the reference IDE's newest configuration folder
//! ("Import Settings" with its folder, "Skip Import", older folders and a
//! folder of one's own under "Other Options") and imports it; the import
//! done, Skip Import, or closing the window goes on to the main window.
//! Without a configuration folder to offer, no window comes: the main
//! window's Welcome tab, with its Theme and Keymap lists, is the first
//! thing a new person sees (the reference's 2026.2 flow, which has no
//! Choose Theme or Choose Keymap page).
//!
//! The decision is stored in `[import]` of the settings (where the
//! settings came from, when, and their snapshot; or `skipped`), so the
//! window comes once; `settings.import-reference` opens it again over a
//! main window, modal to it.
//!
//! Areas import their part of the reference's settings through
//! [`register_importer`]: an importer reads and writes files off the UI
//! thread and returns what to apply on it. The window's own importers
//! ([`import`]) bring the theme and editor colour scheme, the editor font,
//! soft wraps, autosave and file type associations; the workbench adds the
//! keymap and the Search Everywhere preview. The reference's files are
//! never written.
//!
//! An import first copies the reference's data files into the
//! configuration folder (`settings_import::snapshot`), and the importers
//! read that copy; `[import.applied]` lists the importers that ran. An
//! area that comes in a later version registers its importer like the
//! others, and [`pick_up`] runs it once against the snapshot at the next
//! start: values changed after that are never overwritten.

mod import;
mod view;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{App, AppContext as _, Bounds, Global, SharedString, TitlebarOptions, WindowBounds};
use gpui::{WindowHandle, WindowOptions, point, px, size};
use settings::ImportSettings;
use settings_import::pycharm::options::{self, Found};
use ui::notification::Notification;

pub use import::{Applied, Importer, Outcome, Source, nothing};
pub use settings_import::snapshot::Snapshot;
pub use view::FirstRunWindow;

ui::ui_commands! {
    COMMANDS, scope = "FirstRun";
    /// Import a configuration folder.
    ImportFrom { folder: Option<String> } => "first-run.import", "Import Settings",
        "Imports the reference IDE's settings from a configuration folder found (by its name, `PyCharm2026.2`, or its path; without one the newest); the window then closes.",
        r#"{"folder": "PyCharm2026.2"}"#;
    /// Skip the import.
    SkipImport => "first-run.skip", "Skip Import",
        "Goes on without importing (Skip Import): the decision is stored and the window closes.";
    /// The Other Options popup.
    OtherOptions => "first-run.other-options", "Other Options",
        "Shows the Other Options popup of the import page: the older configuration folders and Config or Installation Directory.";
    /// A folder of one's own.
    ChooseFolder { path: Option<String> } => "first-run.choose-folder", "Config or Installation Directory",
        "Imports from a configuration folder chosen in a folder chooser (or the `path` given); a folder without options is refused with an error above the page.",
        r#"{"path": "/path/to/PyCharm2025.3"}"#;
    /// Import the newest folder.
    Continue => "first-run.continue", "Continue",
        "Imports the newest configuration folder found, as its button does (Enter on the import page).";
    /// Close the window.
    Cancel => "first-run.cancel", "Close",
        "Closes the window as its close button does and goes on to the main window; an import not decided yet counts as skipped.";
}

ui::ui_commands! {
    QUERY_COMMANDS, scope = "FirstRun";
    /// The window's state.
    State => "first-run.state", "First-Run Window State",
        "Returns the first-run window's state as JSON: the page (`import`, `progress`), the configuration folders found (main and other), the import's progress, the error shown, the open popup, the decision stored.";
}

ui::ui_commands! {
    WORKBENCH_COMMANDS, scope = "Workbench";
    /// The import again.
    ImportReference => "settings.import-reference", "Import Settings from the Reference IDE\u{2026}",
        "Opens the first-run window over the main window, modal to it: import the reference IDE's settings from a configuration folder, or skip.";
}

/// The window's size (`OnboardingDialog`: 640 by 467).
pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 467.0;
/// Key context of the window.
pub const KEY_CONTEXT: &str = "FirstRun";

/// Shows a balloon in a main window.
pub type Notify = Rc<dyn Fn(Notification, &mut App)>;

/// Runs once the first-run window has closed (the start-up's main window).
pub type Done = Box<dyn FnOnce(&mut App)>;

#[derive(Default)]
struct Hooks {
    importers: Vec<Importer>,
    notify: Option<Notify>,
    /// The balloon of an import made before a main window was open.
    pending: Option<Notification>,
    /// Where to look for the reference's folders instead (tests).
    reference_root: Option<Option<PathBuf>>,
    /// A [`pick_up`] is running.
    picking_up: bool,
}

impl Global for Hooks {}

fn hooks(cx: &mut App) -> &mut Hooks {
    cx.default_global::<Hooks>()
}

/// Registers the commands and the `[import]` and `[file-types]` sections.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, QUERY_COMMANDS);
    ui::command::register(cx, WORKBENCH_COMMANDS);
    crate::store::register::<ImportSettings>(cx);
    crate::store::register::<settings::FileTypeSettings>(cx);
    for importer in import::platform_importers() {
        register_importer(cx, importer);
    }
    view::init(cx);
}

/// Adds an area's importer; importers run in the order they were added.
pub fn register_importer(cx: &mut App, importer: Importer) {
    let h = hooks(cx);
    if !h.importers.iter().any(|i| i.id == importer.id) {
        h.importers.push(importer);
    }
}

/// The ids of the importers, in order.
pub fn importer_ids(cx: &App) -> Vec<&'static str> {
    cx.try_global::<Hooks>()
        .map(|h| h.importers.iter().map(|i| i.id).collect())
        .unwrap_or_default()
}

/// Where the import's balloon goes while a main window is open.
pub fn set_notify(cx: &mut App, notify: Notify) {
    hooks(cx).notify = Some(notify);
}

/// Looks for the reference's configuration folders in `root` from now on
/// (`None`: nowhere), whatever [`Start::reference_root`] says (tests).
pub fn set_reference_root(cx: &mut App, root: Option<PathBuf>) {
    hooks(cx).reference_root = Some(root);
}

/// The balloon of an import made before the main window opened; the
/// caller shows it there.
pub fn take_report(cx: &mut App) -> Option<Notification> {
    hooks(cx).pending.take()
}

/// The first start is not decided yet: neither imported nor skipped.
pub fn undecided(cx: &App) -> bool {
    !crate::store::get::<ImportSettings>(cx).decided()
}

/// How the window opens.
#[derive(Debug, Clone)]
pub struct Start {
    /// The window's title: the product's name.
    pub title: SharedString,
    /// The folder of the reference's configuration folders
    /// ([`options::config_root`]); `None` finds none.
    pub reference_root: Option<PathBuf>,
    /// Give the window the focus (off in the background).
    pub activate: bool,
}

/// Looks for the reference's configuration folders off the UI thread.
fn find(start: &Start, cx: &mut App) -> gpui::Task<Found> {
    let root = match hooks(cx).reference_root.clone() {
        Some(root) => root,
        None => start.reference_root.clone(),
    };
    cx.background_executor().spawn(async move {
        match root {
            Some(root) => options::find_config_folders(&root, SystemTime::now()),
            None => Found::default(),
        }
    })
}

/// Opens the first-run window on its own, before any main window: the
/// folders are looked for first (off the UI thread), then the window opens
/// on the import page. With nothing to import no window opens: the first
/// start counts as skipped and `done` runs at once. `done` runs once,
/// when the window has closed.
pub fn open(start: Start, cx: &mut App, done: impl FnOnce(&mut App) + 'static) {
    let found = find(&start, cx);
    cx.spawn(async move |cx| {
        let found = found.await;
        let _ = cx.update(|cx| {
            if found.is_empty() {
                decide_skipped(cx);
                done(cx);
            } else {
                open_window(start, found, Box::new(done), cx);
            }
        });
    })
    .detach();
}

fn open_window(
    start: Start,
    found: Found,
    done: Done,
    cx: &mut App,
) -> Option<WindowHandle<FirstRunWindow>> {
    let themed = true;
    let content = size(
        px(WIDTH),
        px(HEIGHT + ui::dialog_window::title_height(themed)),
    );
    let mut bounds = Bounds::centered(None, content, cx);
    bounds.origin = bounds.origin.map(|v| v.floor());
    let macos = cfg!(target_os = "macos");
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(start.title.clone()),
            appears_transparent: macos,
            traffic_light_position: macos.then(|| point(px(12.0), px(9.0))),
        }),
        focus: start.activate,
        show: true,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some(start.title.to_lowercase()),
        ..WindowOptions::default()
    };
    let activate = start.activate;
    let handle = cx
        .open_window(options, |window, cx| {
            cx.new(|cx| FirstRunWindow::new(start, found, Some(done), false, window, cx))
        })
        .ok()?;
    if activate {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
    }
    Some(handle)
}

/// `settings.import-reference`: the window over a main window, modal to
/// it; the import's balloon shows in the main window.
pub fn open_over(parent: &mut gpui::Window, start: Start, cx: &mut App) {
    let found = find(&start, cx);
    let parent_handle = parent.window_handle();
    cx.spawn(async move |cx| {
        let found = found.await;
        let _ = parent_handle.update(cx, |_, window, cx| {
            let options = ui::dialog_window::DialogOptions {
                title: start.title.clone(),
                size: size(
                    px(WIDTH),
                    px(HEIGHT + ui::dialog_window::title_height(true)),
                ),
                min_size: None,
                activate: start.activate,
                themed_title: true,
                modal: true,
                key: None,
            };
            ui::dialog_window::open(window, options, cx, |window, cx| {
                cx.new(|cx| FirstRunWindow::new(start, found, None, true, window, cx))
            });
        });
    })
    .detach();
}

/// Runs once, against the snapshot of the import, the importers that are
/// not in `[import.applied]` yet (areas that came after the import), and
/// marks them; their balloon shows in a main window. Nothing happens
/// without an import, a snapshot or such importers. Call it at start-up,
/// after every area registered its importer.
pub fn pick_up(cx: &mut App) {
    let record = crate::store::get::<ImportSettings>(cx);
    let Some(config_dir) = import::config_dir(cx) else {
        return;
    };
    let Some(dir) = record.snapshot_dir(&config_dir) else {
        return;
    };
    let pending: Vec<Importer> = importers(cx)
        .into_iter()
        .filter(|i| !record.is_applied(i.id))
        .collect();
    if pending.is_empty() || hooks(cx).picking_up {
        return;
    }
    hooks(cx).picking_up = true;
    cx.spawn(async move |cx| {
        let read = {
            let (dir, config_dir) = (dir.clone(), config_dir.clone());
            cx.background_executor().spawn(async move {
                dir.is_dir()
                    .then(|| Arc::new(import::recorded_source(&record, dir, config_dir)))
            })
        };
        let source = read.await;
        let mut applied = Vec::new();
        if let Some(source) = &source {
            for importer in &pending {
                let (source, work) = (source.clone(), importer.work);
                let done = cx.background_executor().spawn(async move { work(&source) });
                applied.push((importer.id, done.await));
            }
        }
        let _ = cx.update(|cx| {
            hooks(cx).picking_up = false;
            let Some(source) = source else {
                return;
            };
            // Only while the import read is the one recorded, and for the
            // importers an import meanwhile did not run.
            let mut record = crate::store::get::<ImportSettings>(cx);
            if record.snapshot_dir(&config_dir) != Some(dir) {
                return;
            }
            let mut todo: Vec<Applied> = Vec::new();
            for (id, apply) in applied {
                if !record.is_applied(id) {
                    record.mark_applied(id);
                    todo.push(apply);
                }
            }
            let applied = todo;
            let outcome = import::apply_all(applied, cx);
            crate::store::set(cx, &record);
            if !outcome.imported.is_empty() || !outcome.problems.is_empty() {
                report(import::notification(&source.folder, &outcome), true, cx);
            }
        });
    })
    .detach();
}

/// Records how the first start ended when nothing was recorded yet.
fn decide_skipped(cx: &mut App) {
    if undecided(cx) {
        crate::store::set(cx, &ImportSettings::skipped());
    }
}

/// Shows the import's balloon: in a main window when one is open (and a
/// notifier is set), else later through [`take_report`].
fn report(notification: Notification, main_window_open: bool, cx: &mut App) {
    let notify = hooks(cx).notify.clone();
    match notify {
        Some(notify) if main_window_open => notify(notification, cx),
        _ => hooks(cx).pending = Some(notification),
    }
}

fn importers(cx: &App) -> Vec<Importer> {
    cx.try_global::<Hooks>()
        .map(|h| h.importers.clone())
        .unwrap_or_default()
}
