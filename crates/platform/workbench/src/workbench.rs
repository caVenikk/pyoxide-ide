//! The main window's root view.

use std::path::PathBuf;
use std::rc::Rc;

use editor::Editor;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, Pixels,
    Render, SharedString, Window, canvas, div, prelude::*, px,
};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button, island};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::app::Product;
use crate::chrome;
use crate::commands::{SetTheme, ToggleProject};
use crate::env;

/// Default width of the project tool window.
pub const PROJECT_WIDTH: f32 = 300.0;

/// The main window: toolbar, stripes, project tool window, editor area,
/// status bar.
pub struct Workbench {
    product: Rc<Product>,
    focus: FocusHandle,
    project_visible: bool,
    project_width: Pixels,
    first_frame_reported: bool,
    editor: Option<Entity<Editor>>,
}

impl Workbench {
    pub fn new(product: Rc<Product>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            product,
            focus,
            project_visible: true,
            project_width: px(PROJECT_WIDTH),
            first_frame_reported: false,
            editor: None,
        }
    }

    /// The editor of the open file, if any.
    pub fn active_editor(&self) -> Option<&Entity<Editor>> {
        self.editor.as_ref()
    }

    /// Reads a file off the UI thread and shows it in the editor.
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let read = cx.background_spawn({
            let path = path.clone();
            async move { std::fs::read(&path) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let bytes = read.await;
            let r = this.update_in(cx, |this, window, cx| match bytes {
                Ok(bytes) => match text::decode(&bytes) {
                    Ok(decoded) => {
                        let buffer = text::Buffer::from_decoded(decoded);
                        let editor = cx.new(|cx| {
                            let mut editor = Editor::new(buffer, window, cx);
                            editor.set_path(Some(path.clone()));
                            editor
                        });
                        window.focus(&editor.focus_handle(cx));
                        this.editor = Some(editor);
                        cx.notify();
                    }
                    Err(_) => eprintln!("{} is not a text file", path.display()),
                },
                Err(e) => eprintln!("cannot read {}: {e}", path.display()),
            });
            if let Err(e) = r {
                eprintln!("open_file: {e}");
            }
        })
        .detach();
    }

    pub fn project_visible(&self) -> bool {
        self.project_visible
    }

    /// The state reported by `workbench.state`.
    pub fn state(&self, cx: &App) -> serde_json::Value {
        json!({
            "product": self.product.name,
            "project": null,
            "tool_windows": { "project": { "visible": self.project_visible } },
            "theme": cx.theme().id(),
        })
    }

    fn toggle_project(&mut self, _: &ToggleProject, _: &mut Window, cx: &mut Context<Self>) {
        self.project_visible = !self.project_visible;
        cx.notify();
    }

    fn set_theme(&mut self, action: &SetTheme, _: &mut Window, cx: &mut Context<Self>) {
        match themes::Registry::with_builtin().resolve(&action.id) {
            Ok(theme) => ui::theme::set(cx, theme),
            Err(e) => eprintln!("workbench.set-theme: {e}"),
        }
    }

    fn render_toolbar(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let height = theme.metric(Metric::ToolbarHeight);
        let button = theme.metric(Metric::ToolbarButtonSize);
        let left = if cfg!(target_os = "macos") {
            px(chrome::TRAFFIC_LIGHTS_INSET)
        } else {
            px(8.0)
        };
        div()
            .id("toolbar")
            .debug_selector(|| "toolbar".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(height)
            .pl(left)
            .pr(px(8.0))
            .gap(px(4.0))
            .bg(theme.color(UiRole::Toolbar))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(chrome::project_widget(None, cx))
            .child(div().flex_1())
            .child(chrome::run_widgets(cx))
            .child(div().w(px(8.0)))
            .child(
                icon_button("search", IconName::SEARCH, button, px(20.0), false, cx)
                    .tooltip(Tooltip::text("Search Everywhere", None)),
            )
            .child(
                icon_button("settings", IconName::SETTINGS, button, px(20.0), false, cx)
                    .tooltip(Tooltip::text("Settings", None)),
            )
    }

    fn render_left_stripe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shortcut: SharedString = if cfg!(target_os = "macos") {
            "⌘1".into()
        } else {
            "Alt+1".into()
        };
        let project = chrome::stripe_button(
            "stripe-project",
            IconName::TOOL_PROJECT,
            "Project",
            Some(shortcut),
            self.project_visible,
            cx,
        )
        .on_click(
            cx.listener(|this, _, window, cx| this.toggle_project(&ToggleProject, window, cx)),
        )
        .into_any_element();
        let top = vec![
            project,
            chrome::stripe_button(
                "stripe-commit",
                IconName::TOOL_COMMIT,
                "Commit",
                None,
                false,
                cx,
            )
            .into_any_element(),
            chrome::stripe_button(
                "stripe-structure",
                IconName::TOOL_STRUCTURE,
                "Structure",
                None,
                false,
                cx,
            )
            .into_any_element(),
        ];
        let mut bottom: Vec<_> = self
            .product
            .left_stripe_bottom
            .iter()
            .map(|item| {
                chrome::stripe_button(item.id, item.icon, item.title, None, false, cx)
                    .into_any_element()
            })
            .collect();
        for (id, icon, title) in [
            ("stripe-services", IconName::TOOL_SERVICES, "Services"),
            ("stripe-terminal", IconName::TOOL_TERMINAL, "Terminal"),
            ("stripe-problems", IconName::TOOL_PROBLEMS, "Problems"),
            ("stripe-vcs", IconName::TOOL_VCS, "Version Control"),
        ] {
            bottom.push(chrome::stripe_button(id, icon, title, None, false, cx).into_any_element());
        }
        chrome::stripe("left-stripe", top, bottom, cx)
    }

    fn render_right_stripe(&self, cx: &App) -> impl IntoElement {
        let top = vec![
            chrome::stripe_button(
                "stripe-notifications",
                IconName::TOOL_NOTIFICATIONS,
                "Notifications",
                None,
                false,
                cx,
            )
            .into_any_element(),
        ];
        chrome::stripe("right-stripe", top, Vec::new(), cx)
    }

    fn render_project(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let header_h = theme.metric(Metric::ToolWindowHeaderHeight);
        let action = |id: &'static str, icon: IconName, title: &'static str| {
            icon_button(id, icon, px(24.0), px(16.0), false, cx).tooltip(Tooltip::text(title, None))
        };
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(header_h)
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child("Project"),
                    )
                    .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0))),
            )
            .child(div().flex_1())
            .child(action(
                "project-locate",
                IconName::LOCATE,
                "Select Opened File",
            ))
            .child(action("project-expand", IconName::EXPAND_ALL, "Expand All"))
            .child(action(
                "project-collapse",
                IconName::COLLAPSE_ALL,
                "Collapse All",
            ))
            .child(action(
                "project-options",
                IconName::MORE_VERTICAL,
                "Options",
            ))
            .child(action("project-hide", IconName::HIDE, "Hide"));
        let empty = div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .text_color(theme.color(UiRole::TextMuted))
            .child("No project is open");
        island(cx, UiRole::ToolWindow)
            .id("project-tool-window")
            .debug_selector(|| "project-tool-window".into())
            .flex_none()
            .w(self.project_width)
            .h_full()
            .child(header)
            .child(empty)
    }

    fn render_editor_area(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        if let Some(editor) = &self.editor {
            return island(cx, UiRole::EditorBackground)
                .id("editor-area")
                .debug_selector(|| "editor-area".into())
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(editor.clone())
                .into_any_element();
        }
        let muted = theme.color(UiRole::TextMuted);
        let hint = |text: &'static str, keys: Option<&'static str>| {
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(div().text_color(muted).child(text))
                .children(keys.map(|k| div().text_color(theme.color(UiRole::TextLink)).child(k)))
        };
        let project_keys = if cfg!(target_os = "macos") {
            "⌘1"
        } else {
            "Alt+1"
        };
        island(cx, UiRole::EditorBackground)
            .id("editor-area")
            .debug_selector(|| "editor-area".into())
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(hint("Open a folder to start", None))
            .child(hint("Project tool window", Some(project_keys)))
            .child(hint("Drop files here to open them", None))
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (height, text_size) = chrome::status_metrics(cx);
        div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .h(height)
            .px(px(10.0))
            .bg(theme.color(UiRole::StatusBar))
            .text_size(text_size)
            .text_color(theme.color(UiRole::StatusBarText))
            .child(div().flex().flex_row().items_center())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(self.product.interpreter_label.map(|label| {
                        chrome::status_widget("status-interpreter", label.into(), cx)
                    })),
            )
    }
}

impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let gap = cx.theme().metric(Metric::IslandGap);
        let frame = cx.theme().color(UiRole::TitleBar);
        let first_frame = (!self.first_frame_reported).then(|| {
            self.first_frame_reported = true;
            canvas(
                |_, _, _| (),
                |_, _, _, cx| {
                    let ms = env::since_main_ms();
                    cx.defer(move |cx| {
                        env::marker(&format!("first_frame main_ms={ms:.1}"));
                        if env::bench() == Some(env::Bench::Startup) {
                            cx.quit();
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });
        ui::components::root(cx)
            .id("workbench")
            .key_context("Workbench")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::toggle_project))
            .on_action(cx.listener(Self::set_theme))
            .flex()
            .flex_col()
            .bg(frame)
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.render_left_stripe(cx))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .gap(gap)
                            .when(self.project_visible, |d| d.child(self.render_project(cx)))
                            .child(self.render_editor_area(cx)),
                    )
                    .child(self.render_right_stripe(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(first_frame)
    }
}
