//! The first-run window (ADR 0017, the reference's onboarding dialog of
//! `intellij.ide.startup.importSettings`): at the first start, before the
//! main window, it offers the reference IDE's newest configuration folder
//! ("Import Settings" with its folder, "Skip Import", older folders and a
//! folder of one's own under "Other Options"), imports it, and then lets
//! the person choose the theme and the keymap ("Choose Theme" with Dark and
//! Light, "Choose Keymap" with sample shortcuts), the imported ones
//! preselected. Back and Continue go between the pages; Continue on the
//! last page, or closing the window, goes on to the main window.
//!
//! The decision is stored in `[import]` of the settings (the folder's name
//! or `skipped`), so the window comes once; `settings.import-reference`
//! opens it again over a main window, modal to it.
//!
//! Areas import their part of the reference's settings through
//! [`register_importer`]: an importer reads and writes files off the UI
//! thread and returns what to apply on it. The window's own importers
//! ([`import`]) bring the theme and editor colour scheme, the editor font,
//! soft wraps, autosave and file type associations; the workbench adds the
//! keymap and the Search Everywhere preview, the Python product the
//! interpreters. The keymap page asks the workbench for the keymaps
//! through [`set_keymaps`]. The reference's files are never written.

mod import;
mod view;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use gpui::{App, AppContext as _, Bounds, Global, SharedString, TitlebarOptions, WindowBounds};
use gpui::{WindowHandle, WindowOptions, point, px, size};
use settings::ImportSettings;
use settings_import::pycharm::options::{self, Found};
use ui::notification::Notification;

pub use import::{Applied, Importer, Outcome, Source, nothing};
pub use view::FirstRunWindow;

ui::ui_commands! {
    COMMANDS, scope = "FirstRun";
    /// Import a configuration folder.
    ImportFrom { folder: Option<String> } => "first-run.import", "Import Settings",
        "Imports the reference IDE's settings from a configuration folder found (by its name, `PyCharm2026.2`, or its path; without one the newest), then shows Choose Theme.",
        r#"{"folder": "PyCharm2026.2"}"#;
    /// Skip the import.
    SkipImport => "first-run.skip", "Skip Import",
        "Goes on without importing (Skip Import): Choose Theme comes next and the decision is stored.";
    /// The Other Options popup.
    OtherOptions => "first-run.other-options", "Other Options",
        "Shows the Other Options popup of the import page: the older configuration folders and Config or Installation Directory\u{2026}.";
    /// A folder of one's own.
    ChooseFolder { path: Option<String> } => "first-run.choose-folder", "Config or Installation Directory\u{2026}",
        "Imports from a configuration folder chosen in a folder chooser (or the `path` given); a folder without options is refused with an error above the page.",
        r#"{"path": "/path/to/PyCharm2025.3"}"#;
    /// The page before.
    Back => "first-run.back", "Back",
        "Goes back a page (Choose Keymap to Choose Theme, Choose Theme to the import page).";
    /// The page after.
    Continue => "first-run.continue", "Continue",
        "Goes on: Choose Theme to Choose Keymap, Choose Keymap to the main window (the window closes).";
    /// Close the window.
    Cancel => "first-run.cancel", "Close",
        "Closes the window as its close button does and goes on to the main window; an import not decided yet counts as skipped.";
    /// Dark or Light.
    SelectThemeKind { kind: String } => "first-run.theme-kind", "Dark or Light",
        "Chooses Dark or Light on Choose Theme (`dark`, `light`): the themes of that kind are shown and the last one chosen of them, else the first, becomes the theme.",
        r#"{"kind": "light"}"#;
    /// Choose a theme.
    SelectTheme { id: String } => "first-run.select-theme", "Choose Theme",
        "Makes a theme of Choose Theme by id the theme at once, as clicking its card does.",
        r#"{"id": "islands-light"}"#;
    /// Choose a keymap.
    SelectKeymap { name: String } => "first-run.select-keymap", "Choose Keymap",
        "Makes a keymap of Choose Keymap by name the keymap at once, as clicking its card does.",
        r#"{"name": "VSCode"}"#;
    /// The next card.
    SelectNext => "first-run.select-next", "Next Card",
        "Chooses the card right of the chosen one on Choose Theme or Choose Keymap (Right, Down).";
    /// The card before.
    SelectPrevious => "first-run.select-previous", "Previous Card",
        "Chooses the card left of the chosen one on Choose Theme or Choose Keymap (Left, Up).";
}

ui::ui_commands! {
    QUERY_COMMANDS, scope = "FirstRun";
    /// The window's state.
    State => "first-run.state", "First-Run Window State",
        "Returns the first-run window's state as JSON: the page, the configuration folders found (main and other), the import's progress, the error shown, the cards of Choose Theme or Choose Keymap with the chosen one, the buttons.";
}

ui::ui_commands! {
    WORKBENCH_COMMANDS, scope = "Workbench";
    /// The import again.
    ImportReference => "settings.import-reference", "Import Settings from the Reference IDE\u{2026}",
        "Opens the first-run window over the main window, modal to it: import the reference IDE's settings from a configuration folder (or skip), then choose the theme and the keymap.";
}

/// The window's size (`OnboardingDialog`: 640 by 467).
pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 467.0;
/// Key context of the window.
pub const KEY_CONTEXT: &str = "FirstRun";

/// Shows a balloon in a main window.
pub type Notify = Rc<dyn Fn(Notification, &mut App)>;

/// What the keymap page offers, from the workbench that has the keymaps.
#[derive(Clone, Copy)]
pub struct KeymapChoice {
    /// The keymaps to show, in order.
    pub keymaps: fn(&App) -> Vec<String>,
    /// The shortcuts of commands in a keymap as its card shows them.
    pub shortcuts: fn(keymap: &str, commands: &[&str], &App) -> Vec<Option<String>>,
    /// The keymap in use.
    pub current: fn(&App) -> String,
    /// Makes a keymap the one in use and stores it.
    pub switch: fn(&str, &mut App) -> Result<(), String>,
}

#[derive(Default)]
struct Hooks {
    importers: Vec<Importer>,
    keymaps: Option<KeymapChoice>,
    notify: Option<Notify>,
    /// The balloon of an import made before a main window was open.
    pending: Option<Notification>,
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

/// Sets what the keymap page offers.
pub fn set_keymaps(cx: &mut App, choice: KeymapChoice) {
    hooks(cx).keymaps = Some(choice);
}

/// Where the import's balloon goes while a main window is open.
pub fn set_notify(cx: &mut App, notify: Notify) {
    hooks(cx).notify = Some(notify);
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
    let root = start.reference_root.clone();
    cx.background_executor().spawn(async move {
        match root {
            Some(root) => options::find_config_folders(&root, SystemTime::now()),
            None => Found::default(),
        }
    })
}

/// Opens the first-run window on its own, before any main window: the
/// folders are looked for first (off the UI thread), then the window opens
/// on the import page, or on Choose Theme when there is nothing to import.
/// `done` runs once when the window has closed.
pub fn open(start: Start, cx: &mut App, done: impl FnOnce(&mut App) + 'static) {
    let found = find(&start, cx);
    cx.spawn(async move |cx| {
        let found = found.await;
        let _ = cx.update(|cx| open_window(start, found, Box::new(done), cx));
    })
    .detach();
}

fn open_window(
    start: Start,
    found: Found,
    done: Box<dyn FnOnce(&mut App)>,
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

/// Records how the first start ended when nothing was recorded yet.
fn decide_skipped(cx: &mut App) {
    if undecided(cx) {
        crate::store::set(cx, &ImportSettings::skipped());
    }
}

/// Shows the import's balloon: in a main window when one is open (and a
/// notifier is set), else later through [`take_report`].
fn report(notification: Notification, in_dialog: bool, cx: &mut App) {
    let notify = hooks(cx).notify.clone();
    match notify {
        Some(notify) if in_dialog => notify(notification, cx),
        _ => hooks(cx).pending = Some(notification),
    }
}

fn keymap_choice(cx: &App) -> Option<KeymapChoice> {
    cx.try_global::<Hooks>().and_then(|h| h.keymaps)
}

fn importers(cx: &App) -> Vec<Importer> {
    cx.try_global::<Hooks>()
        .map(|h| h.importers.clone())
        .unwrap_or_default()
}
