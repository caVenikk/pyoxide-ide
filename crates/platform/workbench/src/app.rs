//! Application start-up: what a product supplies, global registrations and
//! the main window.

use std::rc::Rc;
use std::time::Duration;

use code_intel_ui::CodeIntelHook;
use gpui::{
    AnyView, App, AppContext as _, Application, Bounds, Menu, MenuItem, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, point, px, size,
};
use ui::command::CommandError;
use ui::icons::{IconName, IconSvg};

use crate::commands::{self, Quit, SetTheme, ToggleProject};
use crate::env;
use crate::file_types::{FileAssociation, FileType};
use crate::project_panel;
use crate::run::RunHook;
use crate::workbench::Workbench;

/// A tool window button of a stripe that a product adds.
#[derive(Debug, Clone)]
pub struct StripeItem {
    pub id: &'static str,
    pub icon: IconName,
    pub title: &'static str,
}

/// What the product's language adds to Go to File and Search Everywhere.
#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationHooks {
    /// The source roots the product marks on its own in a project that
    /// has not stored its roots yet (ADR 0013; run in the background):
    /// folders below the project folder that hold the code.
    pub source_roots: Option<fn(&std::path::Path) -> Vec<std::path::PathBuf>>,
    /// The location shown after a file's name, from the file name and its
    /// folder relative to its source root (or the project folder), with
    /// `/` separators; `None` shows the folder relative to the project.
    pub file_location: Option<fn(&str, &str) -> Option<String>>,
    /// Files that make the folder holding them a package (it shows the
    /// package icon).
    pub package_markers: &'static [&'static str],
}

/// A product's view in the status bar, right of the platform's widgets
/// (for example the Python interpreter widget). `create` runs once per
/// main window, after the workbench is built; the view may subscribe to
/// [`WorkbenchEvent`](crate::workbench::WorkbenchEvent)s through the
/// workbench handle.
#[derive(Clone)]
pub struct StatusWidget {
    /// The widget's id.
    pub id: &'static str,
    pub create: CreateStatusWidget,
}

/// Creates a [`StatusWidget`]'s view for a main window.
pub type CreateStatusWidget = Rc<dyn Fn(WeakEntity<Workbench>, &mut Window, &mut App) -> AnyView>;

impl std::fmt::Debug for StatusWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusWidget")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// What a product tells the workbench about itself.
#[derive(Debug, Clone, Default)]
pub struct Product {
    /// File types of the product's languages (icons, indentation,
    /// highlighting); they take precedence over the platform's.
    pub file_types: Vec<FileType>,
    /// The product's associations of file names with its types or the
    /// platform's (`uv.lock` is TOML); they replace the built-in ones of
    /// the same mask.
    pub file_associations: Vec<FileAssociation>,
    /// Name shown in the window title and the application menu.
    pub name: &'static str,
    /// Icons of the product (file types, tool windows).
    pub icons: &'static [IconSvg],
    /// Extra tool window buttons in the lower group of the left stripe.
    pub left_stripe_bottom: Vec<StripeItem>,
    /// The product's widgets in the status bar, in order, right of the
    /// platform's widgets.
    pub status_widgets: Vec<StatusWidget>,
    /// File masks Find in Files offers before any was used, the first one
    /// shown in its mask field.
    pub find_file_masks: Vec<&'static str>,
    /// Folders the product excludes from a project on its own, such as a
    /// virtual environment (ADR 0008).
    pub auto_excluded: project::AutoExclusions,
    /// Source roots, locations and package folders for Go to File.
    pub navigation: NavigationHooks,
    /// Registers the product's commands, globals and key bindings; called
    /// at the end of [`init`].
    pub init: Option<fn(&mut App)>,
    /// Runs files of the product's languages (the run widget, the Run tool
    /// window and the `run.*` commands); without it nothing can be run.
    pub run: Option<RunHook>,
    /// Language services of the product's languages (ADR 0010): editor
    /// tabs of file types with a language id get code intelligence; without
    /// it they get none.
    pub code_intel: Option<CodeIntelHook>,
    /// The product's rows of *New* in the project tool window, after
    /// *Directory* (Python File, Python Package).
    pub new_items: Vec<project_panel::NewItem>,
    /// Packages, modules and the kinds of roots of *Mark Directory As* in
    /// the project tool window (ADR 0013).
    pub tree: project_panel::TreeHooks,
}

/// Default size of the main window's content.
pub const WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);
/// Smallest size the main window can be resized to.
pub const WINDOW_MIN_SIZE: (f32, f32) = (640.0, 400.0);
/// Position of the macOS window buttons inside the main toolbar.
pub const TRAFFIC_LIGHTS: (f32, f32) = (14.0, 13.0);

/// Registers fonts, icons, the theme, commands and key bindings. Called once
/// per application (and by tests before opening a window).
pub fn init(cx: &mut App, product: &Product) {
    project::set_auto_exclusions(product.auto_excluded);
    ui::init(cx, env::theme_kind());
    ui::icons::register(cx, product.icons);
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::GO_TO_LINE_COMMANDS);
    ui::command::register(cx, project_panel::COMMANDS);
    ui::command::register(cx, project_panel::FILE_COMMANDS);
    ui::command::register(cx, project_panel::NEW_ITEM_COMMANDS);
    ui::command::register(cx, project_panel::ROOT_COMMANDS);
    ui::command::register(cx, project_panel::TRANSFER_COMMANDS);
    ui::command::register(cx, project_panel::name_popup::COMMANDS);
    ui::command::register(cx, ui::menu::COMMANDS);
    editor::init(cx);
    crate::find_in_files::init(cx, &product.find_file_masks);
    crate::search_everywhere::init(cx);
    crate::file_preview::init(cx);
    crate::recent_files::init(cx);
    crate::navigation::init(cx);
    crate::run::init(cx);
    code_intel_ui::init(cx);
    ui::command::register(cx, crate::dialog::COMMANDS);
    fn workbench(window: &mut gpui::Window) -> Result<gpui::Entity<Workbench>, CommandError> {
        window
            .root::<Workbench>()
            .flatten()
            .ok_or_else(|| CommandError::new("not_available", "not a main window"))
    }
    fn active_editor(
        window: &mut gpui::Window,
        cx: &App,
    ) -> Result<gpui::Entity<editor::Editor>, CommandError> {
        workbench(window)?
            .read(cx)
            .active_editor()
            .cloned()
            .ok_or_else(|| CommandError::new("not_available", "no editor is open"))
    }
    ui::command::register_query(
        cx,
        "workbench.state",
        Rc::new(|_, window, cx| Ok(workbench(window)?.read(cx).state(cx))),
    );
    ui::command::register_query(
        cx,
        "editor.state",
        Rc::new(|_, window, cx| Ok(active_editor(window, cx)?.read(cx).state_json())),
    );
    ui::command::register_query(
        cx,
        "editor.highlights",
        Rc::new(|args, window, cx| {
            let line = args
                .get("line")
                .and_then(|l| l.as_u64())
                .ok_or_else(|| CommandError::invalid_args("editor.highlights: missing `line`"))?;
            let editor = active_editor(window, cx)?;
            let (spans, current) = editor.update(cx, |e, _| {
                (e.highlights(line as usize), e.highlighting_current())
            });
            Ok(serde_json::json!({
                "current": current,
                "spans": spans.iter().map(|(r, role)| serde_json::json!({
                    "start": r.start,
                    "end": r.end,
                    "role": role.name(),
                })).collect::<Vec<_>>(),
            }))
        }),
    );
    ui::command::register_query(
        cx,
        "editor.text",
        Rc::new(|_, window, cx| Ok(active_editor(window, cx)?.read(cx).text().into())),
    );
    ui::command::register_query(
        cx,
        "find.state",
        Rc::new(|_, window, cx| {
            let bar = workbench(window)?
                .read(cx)
                .active_find_bar()
                .cloned()
                .ok_or_else(|| CommandError::new("not_available", "no editor is open"))?;
            Ok(bar.read(cx).state_json(cx))
        }),
    );
    ui::command::register_query(
        cx,
        "find-in-files.state",
        Rc::new(|_, window, cx| {
            let popup = workbench(window)?.read(cx).find_in_files().clone();
            Ok(popup.read(cx).state_json(cx))
        }),
    );
    ui::command::register_query(
        cx,
        "run.state",
        Rc::new(|_, window, cx| Ok(workbench(window)?.read(cx).run_state_json(window, cx))),
    );
    ui::command::register_query(
        cx,
        "recent-files.state",
        Rc::new(|_, window, cx| {
            let popup = workbench(window)?.read(cx).recent_files().clone();
            Ok(popup.read(cx).state_json())
        }),
    );
    ui::command::register_query(
        cx,
        "search-everywhere.state",
        Rc::new(|_, window, cx| {
            let popup = workbench(window)?.read(cx).search_everywhere().clone();
            Ok(popup.read(cx).state_json(cx))
        }),
    );
    fn code_intel(
        window: &mut gpui::Window,
        cx: &App,
    ) -> Result<gpui::Entity<code_intel_ui::CodeIntelView>, CommandError> {
        workbench(window)?
            .read(cx)
            .code_intel()
            .cloned()
            .ok_or_else(|| CommandError::new("not_available", "no language services"))
    }
    ui::command::register_query(
        cx,
        "code.state",
        Rc::new(|_, window, cx| Ok(code_intel(window, cx)?.read(cx).state_json(cx))),
    );
    ui::command::register_query(
        cx,
        "code.servers",
        Rc::new(|_, window, cx| Ok(code_intel(window, cx)?.read(cx).servers_json(cx))),
    );
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Any key ends a double tap of a modifier in progress.
    cx.intercept_keystrokes(|event, _, cx| crate::keymap::key_pressed(&event.keystroke, cx))
        .detach();
    for error in crate::keymap::bind(cx, &crate::keymap::Keymap::platform_default()) {
        eprintln!("keymap: {error}");
    }
    cx.set_menus(vec![
        Menu {
            name: product.name.into(),
            items: vec![
                MenuItem::action("Project", ToggleProject),
                MenuItem::action(
                    "Light Theme",
                    SetTheme {
                        id: themes::builtin::ISLANDS_LIGHT_ID.into(),
                    },
                ),
                MenuItem::action(
                    "Dark Theme",
                    SetTheme {
                        id: themes::builtin::ISLANDS_DARK_ID.into(),
                    },
                ),
                MenuItem::separator(),
                MenuItem::action(format!("Quit {}", product.name), Quit),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::submenu(Menu {
                name: "Active Editor".into(),
                items: vec![MenuItem::action(
                    "Soft-Wrap",
                    editor::commands::ToggleSoftWraps,
                )],
            })],
        },
    ]);
    if let Some(init) = product.init {
        init(cx);
    }
}

/// Window options of the main window: the content extends under a
/// transparent title bar on macOS (the main toolbar takes its place, the
/// window buttons sit inside it); other platforms keep the native frame.
pub fn window_options(cx: &App, product: &Product) -> WindowOptions {
    let content = size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1));
    let mut bounds = Bounds::centered(None, content, cx);
    // A fractional origin makes macOS round the frame and the content size.
    bounds.origin = bounds.origin.map(|v| v.floor());
    let macos = cfg!(target_os = "macos");
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(product.name.into()),
            appears_transparent: macos,
            traffic_light_position: macos
                .then(|| point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
        }),
        focus: !env::background(),
        show: true,
        window_min_size: Some(size(px(WINDOW_MIN_SIZE.0), px(WINDOW_MIN_SIZE.1))),
        app_id: Some(product.name.to_lowercase()),
        ..Default::default()
    }
}

/// Opens the main window.
pub fn open_main_window(cx: &mut App, product: Rc<Product>) -> WindowHandle<Workbench> {
    let options = window_options(cx, &product);
    cx.open_window(options, |window, cx| {
        let workbench = cx.new(|cx| Workbench::new(product.clone(), window, cx));
        // The product's status bar widgets get the finished workbench.
        Workbench::create_status_widgets(&workbench, &product, window, cx);
        workbench
    })
    .expect("open the main window")
}

/// Runs the application with the main window until it quits; `paths`
/// are opened in it.
pub fn run(product: Product, paths: Vec<std::path::PathBuf>) {
    env::mark_main();
    let product = Rc::new(product);
    Application::new().run(move |cx| {
        init(cx, &product);
        // Settings that last across sessions (tests keep the defaults).
        if let Some(file) = crate::find_in_files::settings_file() {
            crate::find_in_files::load_settings(cx, file);
        }
        if let Some(file) = crate::search_everywhere::settings_file() {
            crate::search_everywhere::load_settings(cx, file);
        }
        if let Some(file) = crate::editor_settings::settings_file() {
            crate::editor_settings::load_settings(cx, file);
        }
        if let Some(file) = crate::file_preview::settings_file() {
            crate::file_preview::load_settings(cx, file);
        }
        // Frames follow the display's refresh. After a keystroke the window
        // draws at once instead: the typed text is then ready for the next
        // refresh rather than the one after (ADR 0007, replica/perf.md).
        // Only real windows (the test platform has no native window).
        crate::shot::set_native_windows();
        cx.observe_keystrokes(|_, window, _| crate::shot::request_display(window))
            .detach();
        // `PYOXIDE_NATIVE_POINTER=0` leaves the pointer to GPUI alone
        // (diagnostics, see ui::pointer).
        if std::env::var_os("PYOXIDE_NATIVE_POINTER").is_none_or(|v| v != "0") {
            ui::pointer::enable_native();
        }
        let window = open_main_window(cx, product.clone());
        // The project folder is watched for changes made outside.
        let _ = window.update(cx, |workbench, _, _| {
            let sync = crate::SyncSettings {
                watch: true,
                ..workbench.sync_settings()
            };
            workbench.set_sync_settings(sync);
        });
        // A folder opens as the project, files open in tabs.
        for path in paths {
            let _ = window.update(cx, |workbench, window, cx| {
                if path.is_dir() {
                    workbench.open_folder(path, window, cx)
                } else {
                    let path = std::fs::canonicalize(&path).unwrap_or(path);
                    workbench.open_file(path, window, cx)
                }
            });
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let commands = match env::run_commands() {
            Some(Ok(commands)) => commands,
            Some(Err(e)) => {
                eprintln!("{e}");
                Vec::new()
            }
            None => Vec::new(),
        };
        // The completion measurement takes its own picture (of its popup).
        let shot =
            env::shot_path().filter(|_| !matches!(env::bench(), Some(env::Bench::Completion(_))));
        if !commands.is_empty() || shot.is_some() {
            run_commands_and_shot(window, commands, shot, cx);
        }
        crate::bench::start(window, cx);
        if !env::background() {
            cx.activate(true);
        }
    });
}

/// `PYOXIDE_RUN` and `PYOXIDE_SHOT`: wait until the window is on screen and
/// its files are open, run the commands, then save the window's image and
/// quit.
fn run_commands_and_shot(
    window: WindowHandle<Workbench>,
    commands: Vec<(String, serde_json::Value)>,
    shot: Option<std::path::PathBuf>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(1200))
            .await;
        if !commands.is_empty() {
            // Files open in the background: wait for them (at most 10 s).
            for _ in 0..200 {
                let busy = window.read_with(cx, |w, cx| w.is_busy(cx)).unwrap_or(false);
                if !busy {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
            }
            for (id, args) in commands {
                // A command is available where the last drawn frame has its
                // handler; a covered window may not have drawn the newest
                // state yet.
                let result = cx.update_window(window.into(), |_, window, cx| {
                    window.draw(cx).clear();
                    ui::command::invoke(&id, args, window, cx)
                });
                match result {
                    Ok(Ok(value)) if !value.is_null() => env::marker(&format!("run {id} {value}")),
                    Ok(Ok(_)) => env::marker(&format!("run {id}")),
                    Ok(Err(e)) => eprintln!("PYOXIDE_RUN {id}: {e:?}"),
                    Err(e) => eprintln!("PYOXIDE_RUN {id}: {e}"),
                }
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
            }
        }
        let Some(path) = shot else {
            return;
        };
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::request_display(window)
        });
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
        let save =
            |window: &gpui::Window, path: &std::path::Path| match crate::shot::save(window, path) {
                Ok((w, h)) => env::marker(&format!(
                    "shot {} {w}x{h} scale {}",
                    path.display(),
                    window.scale_factor()
                )),
                Err(e) => eprintln!("PYOXIDE_SHOT failed: {e}"),
            };
        let _ = cx.update_window(window.into(), |_, window, _| save(window, &path));
        // An open dialog window is saved next to it (`<name>-dialog.png`),
        // a popup open over the dialog too (`<name>-popup.png`).
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut parent: gpui::AnyWindowHandle = window.into();
        for suffix in ["dialog", "popup"] {
            let child = cx
                .update(|cx| ui::dialog_window::child_of(parent, cx))
                .ok()
                .flatten();
            let Some(child) = child else {
                break;
            };
            let child_path = path.with_file_name(format!("{stem}-{suffix}.png"));
            let _ = cx.update_window(child, |_, window, _| crate::shot::request_display(window));
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = cx.update_window(child, |_, window, _| {
                save(window, &child_path);
                let b = window.bounds();
                env::marker(&format!(
                    "window {suffix} at {} {} size {} {}",
                    f32::from(b.origin.x),
                    f32::from(b.origin.y),
                    f32::from(b.size.width),
                    f32::from(b.size.height)
                ));
            });
            parent = child;
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
