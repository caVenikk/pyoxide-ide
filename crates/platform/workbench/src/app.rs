//! Application start-up: what a product supplies, global registrations and
//! the main window.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WindowBounds, WindowHandle, WindowOptions, point, px, size,
};
use ui::command::CommandError;
use ui::icons::{IconName, IconSvg};

use crate::commands::{self, Quit, SetTheme, ToggleProject};
use crate::env;
use crate::file_types::FileType;
use crate::project_panel;
use crate::workbench::Workbench;

/// A tool window button of a stripe that a product adds.
#[derive(Debug, Clone)]
pub struct StripeItem {
    pub id: &'static str,
    pub icon: IconName,
    pub title: &'static str,
}

/// What a product tells the workbench about itself.
#[derive(Debug, Clone, Default)]
pub struct Product {
    /// File types of the product's languages (icons, indentation,
    /// highlighting); they take precedence over the platform's.
    pub file_types: Vec<FileType>,
    /// Name shown in the window title and the application menu.
    pub name: &'static str,
    /// Icons of the product (file types, tool windows).
    pub icons: &'static [IconSvg],
    /// Extra tool window buttons in the lower group of the left stripe.
    pub left_stripe_bottom: Vec<StripeItem>,
    /// Text of the interpreter widget in the status bar.
    pub interpreter_label: Option<&'static str>,
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
    ui::init(cx, env::theme_kind());
    ui::icons::register(cx, product.icons);
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, project_panel::COMMANDS);
    editor::init(cx);
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
        "editor.text",
        Rc::new(|_, window, cx| Ok(active_editor(window, cx)?.read(cx).text().into())),
    );
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-1", ToggleProject, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-1", ToggleProject, None),
    ]);
    cx.set_menus(vec![Menu {
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
    }]);
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
        cx.new(|cx| Workbench::new(product, window, cx))
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
        let window = open_main_window(cx, product.clone());
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
        if let Some(path) = env::shot_path() {
            save_shot_and_quit(window, path, cx);
        }
        if !env::background() {
            cx.activate(true);
        }
    });
}

/// `PYOXIDE_SHOT`: wait until the window is on screen and settled, save its
/// image, quit.
fn save_shot_and_quit(window: WindowHandle<Workbench>, path: std::path::PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(1200))
            .await;
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::request_display(window)
        });
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
        let _ = cx.update_window(window.into(), |_, window, _| {
            match crate::shot::save(window, &path) {
                Ok((w, h)) => env::marker(&format!(
                    "shot {} {w}x{h} scale {}",
                    path.display(),
                    window.scale_factor()
                )),
                Err(e) => eprintln!("PYOXIDE_SHOT failed: {e}"),
            }
        });
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
