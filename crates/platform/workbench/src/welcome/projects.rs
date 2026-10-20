//! The Welcome screen's left panel, "Projects and Files" (the reference's
//! `WelcomeScreenLeftPanel` of `intellij.platform.ide.nonModalWelcomeScreen`,
//! IntelliJ Community, Apache-2.0): what the project tool window shows
//! while no project is open. A column of buttons (Open…, New…, Clone
//! Repository…; `createToolWindowWelcomeScreenVerticalToolbar`), a line,
//! the "Search projects" field, a line, and the recent projects (none are
//! kept yet: "No recent projects", a third of the way down, where
//! `StatusText` puts it).
//!
//! Metrics of the reference: the buttons 34 high, 8 apart, 20 from the
//! sides and 8 below the top, corners of 8, the icon 14 from the left and
//! 12 before the label (`DisclosureButton`, `DarculaDisclosureButtonUI`);
//! their background white at 7 % (dark themes) or black at 4 %, 5 % more
//! under the mouse. The line 27 below the last button, the search row 38
//! high.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, Focusable as _, Hsla, IntoElement, Render,
    SharedString, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::commands::OpenFolder;

/// `welcome.screen.project.view.title` (`NonModalWelcomeScreenBundle`).
pub const TITLE: &str = "Projects and Files";
/// `action.Tabbed.WelcomeScreen.NonModal.OpenProject.text`.
pub const OPEN: &str = "Open\u{2026}";
/// `action.NonModalWelcomeScreen.LeftTabActions.New.Action.text`.
pub const NEW: &str = "New\u{2026}";
/// `action.ProjectFromVersionControl.ProjectWidgetPopup.text`.
pub const CLONE: &str = "Clone Repository\u{2026}";
/// The search field's placeholder.
pub const SEARCH: &str = "Search projects";
/// `welcome.screen.no.recent.projects`.
pub const NO_RECENT: &str = "No recent projects";
/// The tooltip of what has nothing behind it yet.
pub const NOT_AVAILABLE: &str = "Not available yet";

const BUTTON_HEIGHT: f32 = 34.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_RADIUS: f32 = 8.0;
const BUTTONS_X: f32 = 20.0;
const BUTTONS_TOP: f32 = 8.0;
const BUTTON_ICON_X: f32 = 14.0;
const BUTTON_ICON_GAP: f32 = 12.0;
const LINE_TOP: f32 = 27.0;
const SEARCH_HEIGHT: f32 = 38.0;
const SEARCH_ICON_X: f32 = 26.0;
const SEARCH_ICON_GAP: f32 = 8.0;

/// A button of the column: its label, icon and what it runs (none yet:
/// drawn disabled, with [`NOT_AVAILABLE`] as its tooltip).
#[derive(Clone, Copy)]
struct Button {
    label: &'static str,
    icon: IconName,
    enabled: bool,
}

/// Open… (`WelcomeScreen.OpenDirectoryProject`: the folder icon on the
/// non-modal Welcome screen), New… (`General.Add`) and Clone Repository…
/// (`General.Vcs`). New… lists the New Project wizard and the new file
/// kinds in the reference, Clone Repository… opens its clone dialog: we
/// have neither yet.
const BUTTONS: [Button; 3] = [
    Button {
        label: OPEN,
        icon: IconName::FOLDER,
        enabled: true,
    },
    Button {
        label: NEW,
        icon: IconName::ADD,
        enabled: false,
    },
    Button {
        label: CLONE,
        icon: IconName::VCS,
        enabled: false,
    },
];

pub struct WelcomeProjects {
    search: Entity<editor::Editor>,
}

impl WelcomeProjects {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = editor::field::new("", Some(SEARCH.into()), true, window, cx);
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self { search }
    }

    /// What the panel shows, for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "title": TITLE,
            "buttons": BUTTONS.iter().map(|b| json!({
                "label": b.label,
                "enabled": b.enabled,
            })).collect::<Vec<_>>(),
            "search": self.search.read(cx).text(),
            "projects": Vec::<Json>::new(),
            "empty_text": NO_RECENT,
        })
    }

    /// White (dark themes) or black at `dark` or `light` opacity.
    fn wash(dark: f32, light: f32, cx: &App) -> Hsla {
        if cx.theme().is_dark() {
            gpui::white().opacity(dark)
        } else {
            gpui::black().opacity(light)
        }
    }

    fn render_button(ix: usize, button: Button, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let text = if button.enabled {
            theme.color(UiRole::Text)
        } else {
            theme.color(UiRole::TextDisabled)
        };
        let label = button.label;
        div()
            .id(("welcome-projects-button", ix))
            .debug_selector(move || format!("welcome-projects-button:{label}"))
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(BUTTON_HEIGHT))
            .pl(px(BUTTON_ICON_X))
            .pr(px(12.0))
            .gap(px(BUTTON_ICON_GAP))
            .rounded(px(BUTTON_RADIUS))
            .bg(Self::wash(0.07, 0.043, cx))
            .text_color(text)
            .child(
                div()
                    .flex_none()
                    .when(!button.enabled, |d| {
                        d.opacity(ui::components::DISABLED_ALPHA)
                    })
                    .child(Icon::new(button.icon).size(px(16.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .when(button.enabled, |d| {
                d.hover(|s| s.bg(Self::wash(0.12, 0.085, cx)))
                    .active(|s| s.bg(Self::wash(0.17, 0.09, cx)))
                    .on_click(|_: &ClickEvent, window, cx| {
                        window.dispatch_action(Box::new(OpenFolder { path: None }), cx)
                    })
            })
            .when(!button.enabled, |d| {
                d.tooltip(Tooltip::text(NOT_AVAILABLE, None))
            })
            .into_any_element()
    }
}

impl Render for WelcomeProjects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let line = theme.color(UiRole::Divider);
        let muted = theme.color(UiRole::TextMuted);
        let rule = || div().flex_none().h(px(1.0)).w_full().bg(line);
        let search_focus = self.search.focus_handle(cx);
        let empty: SharedString = NO_RECENT.into();
        div()
            .id("welcome-projects")
            .debug_selector(|| "welcome-projects".into())
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .child(rule())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .pt(px(BUTTONS_TOP))
                    .px(px(BUTTONS_X))
                    .gap(px(BUTTON_GAP))
                    .children(
                        BUTTONS
                            .iter()
                            .enumerate()
                            .map(|(ix, b)| Self::render_button(ix, *b, cx)),
                    ),
            )
            .child(rule().mt(px(LINE_TOP)))
            .child(
                div()
                    .id("welcome-projects-search")
                    .debug_selector(|| "welcome-projects-search".into())
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(px(SEARCH_HEIGHT))
                    .pl(px(SEARCH_ICON_X))
                    .pr(px(BUTTONS_X))
                    .gap(px(SEARCH_ICON_GAP))
                    .on_click(move |_: &ClickEvent, window, _| window.focus(&search_focus))
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted)
                            .child(Icon::new(IconName::SEARCH).size(px(16.0))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(editor::field_line_height_for(true, window, cx))
                            .overflow_hidden()
                            .child(self.search.clone()),
                    ),
            )
            .child(rule())
            // The empty list's text a third of the way down.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .items_center()
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .debug_selector(|| "welcome-projects-empty".into())
                            .text_color(muted)
                            .child(empty),
                    )
                    .child(div().flex_1())
                    .child(div().flex_1()),
            )
    }
}
