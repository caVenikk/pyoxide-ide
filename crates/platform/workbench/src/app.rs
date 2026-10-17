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

use crate::commands::{self, Quit, ToggleProject};
use crate::env;
use crate::file_types::{FileAssociation, FileType};
use crate::project_panel;
use crate::run::RunHook;
use crate::terminal::TerminalHook;
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
    /// Debugging the product's programs (ADR 0015): the breakpoint types
    /// and where breakpoints can be put, the debug adapter; without it
    /// there are no breakpoints.
    pub debug: Option<debugger_ui::DebugHook>,
    /// The product's rows of *New* in the project tool window, after
    /// *Directory* (Python File, Python Package).
    pub new_items: Vec<project_panel::NewItem>,
    /// Packages, modules and the kinds of roots of *Mark Directory As* in
    /// the project tool window (ADR 0013).
    pub tree: project_panel::TreeHooks,
    /// The product's pages of the Settings window (ADR 0017).
    pub settings_pages: Vec<settings_ui::PageSpec>,
    /// What the product adds to terminal sessions (ADR 0014): environment
    /// variables and scripts to source, such as a virtual environment's
    /// activation.
    pub terminal: Option<TerminalHook>,
}

/// Default size of the main window's content.
pub const WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);
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
    ui::command::register(cx, crate::notifications::COMMANDS);
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
    crate::simulate::init(cx);
    crate::recent_files::init(cx);
    crate::navigation::init(cx);
    crate::run::init(cx);
    terminal_ui::init(cx);
    crate::workbench::terminal_dev::init(cx);
    code_intel_ui::init(cx);
    vcs_ui::init(cx);
    debugger_ui::init(cx);
    ui::command::register(cx, crate::dialog::COMMANDS);
    crate::settings::init(cx, &product.settings_pages);
    crate::terminal::init(cx, product.terminal.clone());
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
        "debug.state",
        Rc::new(|_, window, cx| Ok(workbench(window)?.read(cx).debug_state(cx))),
    );
    ui::command::register_query(
        cx,
        "vcs.state",
        Rc::new(|_, window, cx| Ok(workbench(window)?.read(cx).state(cx)["vcs"].clone())),
    );
    ui::command::register_query(
        cx,
        "workbench.layout",
        Rc::new(|_, window, cx| {
            let workbench = workbench(window)?;
            let viewport = window.viewport_size();
            let mut layout = workbench.read(cx).main_layout(window, cx).to_json();
            layout["window"] = serde_json::json!({
                "width": f32::from(viewport.width),
                "height": f32::from(viewport.height),
            });
            Ok(layout)
        }),
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
    // Changes not applied in the Settings window are asked about first.
    cx.on_action(|_: &Quit, cx| {
        settings_ui::leave::resolve(settings_ui::leave::Leaving::Application, cx, |cx| cx.quit())
    });
    // Any key ends a double tap of a modifier in progress.
    cx.intercept_keystrokes(|event, _, cx| crate::keymap::key_pressed(&event.keystroke, cx))
        .detach();
    // The platform's keymap; the stored one replaces it once the settings
    // are read. The menus show the shortcuts of the keymap in use.
    crate::keymap::active::init(cx);
    let name = product.name;
    let menus = move |cx: &mut App| cx.set_menus(app_menus(name));
    menus(cx);
    crate::keymap::active::set_menus(cx, Rc::new(menus));
    if let Some(init) = product.init {
        init(cx);
    }
}

/// The application menus (macOS menu bar).
fn app_menus(name: &'static str) -> Vec<Menu> {
    vec![
        Menu {
            name: name.into(),
            items: vec![
                MenuItem::action(
                    "Settings\u{2026}",
                    settings_ui::commands::OpenSettings {
                        page: None,
                        filter: None,
                    },
                ),
                MenuItem::separator(),
                MenuItem::action("Project", ToggleProject),
                MenuItem::separator(),
                MenuItem::action(format!("Quit {name}"), Quit),
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
    ]
}

/// Window options of the main window: the content extends under a
/// transparent title bar on macOS (the main toolbar takes its place, the
/// window buttons sit inside it); other platforms keep the native frame.
pub fn window_options(cx: &App, product: &Product) -> WindowOptions {
    let (width, height) = env::window_size().unwrap_or(WINDOW_SIZE);
    let content = size(px(width), px(height));
    // Smaller still when `PYOXIDE_WINDOW_SIZE` asks for it.
    let min = crate::layout::window_min_size(cx);
    let min_size = size(min.width.min(content.width), min.height.min(content.height));
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
        window_min_size: Some(min_size),
        app_id: Some(product.name.to_lowercase()),
        ..Default::default()
    }
}

/// Opens the main window.
pub fn open_main_window(cx: &mut App, product: Rc<Product>) -> WindowHandle<Workbench> {
    let options = window_options(cx, &product);
    cx.open_window(options, |window, cx| {
        let workbench = cx.new(|cx| Workbench::new(product.clone(), window, cx));
        // Autosave and sync follow the application settings.
        workbench.update(cx, |w, cx| {
            w.set_save_settings(settings_ui::store::get(cx));
            w.set_sync_settings(crate::SyncSettings {
                watch: w.sync_settings().watch,
                ..settings_ui::store::get(cx)
            });
        });
        // The product's status bar widgets get the finished workbench.
        Workbench::create_status_widgets(&workbench, &product, window, cx);
        // Closing the window with changes not applied in its Settings
        // window asks first.
        window.on_window_should_close(cx, |window, cx| {
            settings_ui::leave::can_close_parent(window.window_handle(), cx)
        });
        workbench
    })
    .expect("open the main window")
}

/// Runs the application with the main window until it quits; `paths`
/// are opened in it. At the first start the first-run window comes before
/// the main window (`settings_ui::first_run`, see [`env::first_run`]).
pub fn run(product: Product, paths: Vec<std::path::PathBuf>) {
    env::mark_main();
    let product = Rc::new(product);
    Application::new().run(move |cx| {
        init(cx, &product);
        // Settings that last across sessions (tests keep the defaults).
        let file = ::settings::settings_file();
        let settings_exist = file.as_ref().is_some_and(|f| f.exists());
        if let Some(file) = file {
            settings_ui::store::load(cx, file);
        }
        // The imported themes and the theme stored, or the one
        // `PYOXIDE_THEME` asks for in this run.
        settings_ui::theme_switch::load(cx, ::settings::themes_dir(), env::theme_override());
        // The imported keymaps and the keymap stored.
        crate::keymap::active::load(cx, ::settings::keymaps_dir());
        if let Some(dir) = ::settings::config_dir() {
            settings_ui::window_state::load(cx, dir.join(settings_ui::window_state::FILE));
        }
        if let Some(file) = crate::find_in_files::settings_file() {
            crate::find_in_files::load_settings(cx, file);
        }
        if let Some(file) = crate::search_everywhere::settings_file() {
            crate::search_everywhere::load_settings(cx, file);
        }
        if let Some(file) = crate::file_preview::settings_file() {
            crate::file_preview::load_settings(cx, file);
        }
        // Frames follow the display's refresh. After a keystroke the window
        // draws at once instead: the typed text is then ready for the next
        // refresh rather than the one after (ADR 0007, replica/perf.md).
        // Only real windows (the test platform has no native window).
        // Text typed into a terminal is drawn when its echo is back from
        // the shell (the session view's `Redraw`): a frame drawn before it
        // would take the next refresh, and the echo would wait for the one
        // after (Core Animation shows one such frame per refresh).
        crate::shot::set_native_windows();
        cx.observe_keystrokes(|event, window, _| {
            let into_terminal = event.action.is_none()
                && event
                    .context_stack
                    .iter()
                    .any(|c| c.contains(terminal_ui::KEY_CONTEXT));
            if !into_terminal {
                crate::shot::request_display(window)
            }
        })
        .detach();
        // `PYOXIDE_NATIVE_POINTER=0` leaves the pointer to GPUI alone
        // (diagnostics, see ui::pointer).
        if std::env::var_os("PYOXIDE_NATIVE_POINTER").is_none_or(|v| v != "0") {
            ui::pointer::enable_native();
        }
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
        if !env::first_run(settings_exist) {
            open_main(product.clone(), paths, commands, shot, cx);
            return;
        }
        let scripted = !commands.is_empty() || shot.is_some();
        let start = crate::settings::first_run_start(product.name);
        let main = product.clone();
        settings_ui::first_run::open(start, cx, move |cx| {
            open_main(main, paths, Vec::new(), None, cx)
        });
        if scripted {
            run_first_run_script(commands, shot, cx);
        }
    });
}

/// Opens the main window with `paths`, quits with its last window, runs
/// `PYOXIDE_RUN` and `PYOXIDE_SHOT` there, and shows the balloon of an
/// import the first-run window made.
fn open_main(
    product: Rc<Product>,
    paths: Vec<std::path::PathBuf>,
    commands: Vec<(String, serde_json::Value)>,
    shot: Option<std::path::PathBuf>,
    cx: &mut App,
) {
    let window = open_main_window(cx, product);
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
    if let Some(report) = settings_ui::first_run::take_report(cx) {
        let _ = window.update(cx, |workbench, _, cx| workbench.notify(report, cx));
    }
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    if !commands.is_empty() || shot.is_some() {
        run_commands_and_shot(window, commands, shot, cx);
    }
    crate::bench::start(window, cx);
    if !env::background() {
        cx.activate(true);
    }
}

/// `PYOXIDE_RUN` and `PYOXIDE_SHOT` of a run that shows the first-run
/// window: the commands run in it; the picture is of it while it is open
/// (its popup next to it, `<name>-popup.png`), else of the main window
/// that followed it. Then the application quits.
fn run_first_run_script(
    commands: Vec<(String, serde_json::Value)>,
    shot: Option<std::path::PathBuf>,
    cx: &mut App,
) {
    fn first_run_window(cx: &App) -> Option<gpui::AnyWindowHandle> {
        cx.windows().into_iter().find(|w| {
            w.downcast::<settings_ui::first_run::FirstRunWindow>()
                .is_some()
        })
    }
    cx.spawn(async move |cx| {
        let executor = cx.background_executor().clone();
        let pause = move |ms| executor.timer(Duration::from_millis(ms));
        pause(1200).await;
        for (id, args) in commands {
            let target = cx
                .update(|cx| {
                    first_run_window(cx).or_else(|| {
                        cx.windows()
                            .iter()
                            .find_map(|w| w.downcast::<Workbench>().map(Into::into))
                    })
                })
                .ok()
                .flatten();
            let Some(target) = target else {
                break;
            };
            let result = cx.update_window(target, |_, window, cx| {
                window.draw(cx).clear();
                ui::command::invoke(&id, args, window, cx)
            });
            match result {
                Ok(Ok(value)) if !value.is_null() => env::marker(&format!("run {id} {value}")),
                Ok(Ok(_)) => env::marker(&format!("run {id}")),
                Ok(Err(e)) => eprintln!("PYOXIDE_RUN {id}: {e:?}"),
                Err(e) => eprintln!("PYOXIDE_RUN {id}: {e}"),
            }
            pause(400).await;
        }
        if let Some(path) = shot {
            pause(600).await;
            let target = cx
                .update(|cx| {
                    first_run_window(cx).or_else(|| {
                        cx.windows()
                            .iter()
                            .find_map(|w| w.downcast::<Workbench>().map(Into::into))
                    })
                })
                .ok()
                .flatten();
            if let Some(target) = target {
                let _ =
                    cx.update_window(target, |_, window, _| crate::shot::request_display(window));
                pause(300).await;
                let _ = cx.update_window(target, |_, window, _| save_shot(window, &path));
                let child = cx
                    .update(|cx| ui::dialog_window::child_of(target, cx))
                    .ok()
                    .flatten();
                if let Some(child) = child {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let child_path = path.with_file_name(format!("{stem}-popup.png"));
                    let _ = cx
                        .update_window(child, |_, window, _| crate::shot::request_display(window));
                    pause(300).await;
                    let _ = cx.update_window(child, |_, window, _| save_shot(window, &child_path));
                }
            }
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Saves a window's picture and prints its marker.
fn save_shot(window: &gpui::Window, path: &std::path::Path) {
    match crate::shot::save(window, path) {
        Ok((w, h)) => env::marker(&format!(
            "shot {} {w}x{h} scale {}",
            path.display(),
            window.scale_factor()
        )),
        Err(e) => eprintln!("PYOXIDE_SHOT failed: {e}"),
    }
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
                // Dialogs and popups open over the window draw what the
                // command changed too: the next command may need their
                // layout (a list opens under the combo it measured).
                let mut parent: gpui::AnyWindowHandle = window.into();
                while let Some(child) = cx
                    .update(|cx| ui::dialog_window::child_of(parent, cx))
                    .ok()
                    .flatten()
                {
                    let _ = cx
                        .update_window(child, |_, window, _| crate::shot::request_display(window));
                    parent = child;
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
