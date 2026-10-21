//! The first-run window's pages, as the reference's onboarding dialog lays
//! them out (IntelliJ Community, Apache-2.0: `ProductChooserPage`,
//! `ImportProgressPage`, `UiUtils`):
//!
//! - Import Settings: the title (`JBFont.h1`) 26 above a column of 280 by
//!   40 buttons 8 apart (the newest folder, then Skip Import as a link
//!   button), and Other Options at the bottom right;
//! - Importing settings: the title, "Importing from …", a 280 wide bar.
//!
//! The import done, or skipped, the window closes: the theme and the
//! keymap are chosen on the main window's Welcome tab (the reference's
//! 2026.2 flow has no Choose Theme or Choose Keymap page).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context, FocusHandle, Focusable,
    FontWeight, Hsla, IntoElement, PathPromptOptions, Pixels, Render, SharedString, Task, Window,
    canvas, div, prelude::*, px, rgb,
};
use serde_json::{Value as Json, json};
use settings::ImportSettings;
use settings_import::pycharm::options::{ConfigFolder, Found};
use themes::UiRole;
use ui::form::MenuPopup;
use ui::icons::{Icon, IconName};
use ui::menu::{MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::import::{self, Applied};
use super::{
    Cancel, ChooseFolder, Continue, Done, ImportFrom, KEY_CONTEXT, OtherOptions, SkipImport, Start,
};
use crate::commands::OpenSettings;
use crate::store;

/// Strings of the reference (`ImportSettingsBundle`, `BootstrapBundle`).
pub const IMPORT_TITLE: &str = "Import Settings";
pub const SKIP_IMPORT: &str = "Skip Import";
pub const OTHER_OPTIONS: &str = "Other Options";
pub const LOCAL_SETTINGS: &str = "Local Settings";
pub const CUSTOM_DIR: &str = "Config or Installation Directory";
pub const IMPORTING: &str = "Importing settings\u{2026}";
/// The products of the main choices under one button.
pub const REFERENCE: &str = "Reference IDE";

/// Metrics (`UiUtils`, `ProductChooserPage`).
const BUTTON_WIDTH: f32 = 280.0;
const BUTTON_HEIGHT: f32 = 40.0;
const BUTTON_GAP: f32 = 4.0;
/// The icons of the folders' rows (`ProductChooserRenderer`, 20 by 20).
const ROW_ICON: f32 = 20.0;
const TITLE_GAP: f32 = 26.0;
const SOUTH_HEIGHT: f32 = 47.0;
/// `JBFont.h1()`: the label font plus 9, bold.
const H1_PLUS: f32 = 9.0;
/// `OnboardingDialogButtons.BUTTON_HOVER_BORDER_COLOR`.
const HOVER_BORDER_LIGHT: u32 = 0xa8adbd;
const HOVER_BORDER_DARK: u32 = 0x6f737a;

pub(super) fn init(cx: &mut App) {
    // Commands that the window answers come through its handlers; the
    // state query reads the window it runs in.
    ui::command::register_query(
        cx,
        "first-run.state",
        Rc::new(|_, window, cx| {
            let view = window
                .root::<FirstRunWindow>()
                .flatten()
                .or_else(|| {
                    let dialog = ui::dialog_window::child_of(window.window_handle(), cx)?;
                    dialog.downcast::<FirstRunWindow>()?.entity(cx).ok()
                })
                .ok_or_else(|| {
                    ui::command::CommandError::new("not_available", "no first-run window")
                })?;
            Ok(view.read(cx).state_json(cx))
        }),
    );
}

/// A page of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Import,
    Progress,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Import => "import",
            Page::Progress => "progress",
        }
    }
}

pub struct FirstRunWindow {
    title: SharedString,
    found: Found,
    page: Page,
    /// The folder being imported or imported last.
    importing: Option<ConfigFolder>,
    /// Importers done and all of them.
    progress: (usize, usize),
    /// An error over the import page (a folder that is not a
    /// configuration folder).
    error: Option<SharedString>,
    popup: Option<MenuPopup>,
    other_anchor: Rc<Cell<Bounds<Pixels>>>,
    main_anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    /// The window the view is the content of.
    window: AnyWindowHandle,
    /// Runs once the window closed (the start-up's main window).
    done: Option<Done>,
    /// Open over a main window (`settings.import-reference`).
    in_dialog: bool,
    activate: bool,
    work: Option<Task<()>>,
}

impl FirstRunWindow {
    pub(super) fn new(
        start: Start,
        found: Found,
        done: Option<Done>,
        in_dialog: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        // The window closed by its title bar goes on as Skip Import would.
        if !in_dialog {
            window.on_window_should_close(cx, |window, cx| {
                if let Some(view) = window.root::<FirstRunWindow>().flatten() {
                    view.update(cx, |view, cx| view.finished(cx));
                }
                true
            });
        }
        Self {
            title: start.title,
            found,
            page: Page::Import,
            importing: None,
            progress: (0, 0),
            error: None,
            popup: None,
            other_anchor: Rc::default(),
            main_anchor: Rc::default(),
            focus,
            window: window.window_handle(),
            done,
            in_dialog,
            activate: start.activate,
            work: None,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        self.close_popup(cx);
        self.error = None;
        self.page = page;
        cx.notify();
    }

    // ---- import -----------------------------------------------------------------

    /// The folder `name` names (a folder's name or path); `None`: the
    /// newest.
    fn folder(&self, name: Option<&str>) -> Option<ConfigFolder> {
        match name {
            None => self
                .found
                .main
                .first()
                .or(self.found.other.first())
                .cloned(),
            Some(name) => self
                .found
                .all()
                .find(|f| f.name == name || f.path.to_string_lossy() == name)
                .cloned()
                .or_else(|| {
                    settings_import::pycharm::options::custom_folder(std::path::Path::new(name))
                }),
        }
    }

    fn on_import(&mut self, a: &ImportFrom, _: &mut Window, cx: &mut Context<Self>) {
        match self.folder(a.folder.as_deref()) {
            Some(folder) => self.import(folder, cx),
            None => {
                self.error = Some(unrecognized(a.folder.as_deref().unwrap_or_default()).into());
                cx.notify();
            }
        }
    }

    /// Imports `folder` in the background, then closes the window.
    fn import(&mut self, folder: ConfigFolder, cx: &mut Context<Self>) {
        if self.page == Page::Progress {
            return;
        }
        let importers = super::importers(cx);
        let config_dir = import::config_dir(cx);
        self.importing = Some(folder.clone());
        self.progress = (0, importers.len());
        self.go(Page::Progress, cx);
        let in_dialog = self.in_dialog;
        self.work = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { import::prepare(folder, config_dir) })
                .await;
            let source = Arc::new(prepared.source);
            let mut applied: Vec<Applied> = Vec::new();
            for (n, importer) in importers.iter().enumerate() {
                let source = source.clone();
                let work = importer.work;
                applied.push(
                    cx.background_executor()
                        .spawn(async move { work(&source) })
                        .await,
                );
                let _ = this.update(cx, |this, cx| {
                    this.progress.0 = n + 1;
                    cx.notify();
                });
            }
            let _ = cx.update(|cx| {
                let mut outcome = import::apply_all(applied, cx);
                outcome.problems.extend(prepared.problem);
                let ran: Vec<&str> = importers.iter().map(|i| i.id).collect();
                let record = import::record(&source.folder, prepared.snapshot.as_deref(), &ran);
                store::set(cx, &record);
                super::report(
                    import::notification(&source.folder, &outcome),
                    in_dialog,
                    cx,
                );
                cx.refresh_windows();
            });
            let _ = this.update(cx, |this, cx| {
                this.work = None;
                this.close(cx);
            });
        }));
    }

    fn on_skip(&mut self, _: &SkipImport, _: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Import {
            return;
        }
        super::decide_skipped(cx);
        self.close(cx);
    }

    fn on_other_options(&mut self, _: &OtherOptions, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Import {
            return;
        }
        let mut items = Vec::new();
        let other: Vec<&ConfigFolder> = self.found.other.iter().collect();
        if !other.is_empty() {
            items.push(MenuItem::header(LOCAL_SETTINGS));
            for folder in other {
                items.push(
                    product_row(folder).description(folder.path.to_string_lossy().into_owned()),
                );
            }
            items.push(MenuItem::separator());
        }
        items.push(
            MenuItem::action(CUSTOM_DIR, ChooseFolder { path: None })
                .icon(IconName::CONFIG_FILE)
                .icon_size(ROW_ICON),
        );
        let position = self.other_anchor.get().bottom_left() + gpui::point(px(0.0), px(4.0));
        self.open_popup(items, position, window, cx);
    }

    /// The lists are at least as wide as the buttons
    /// (`UiUtils.DEFAULT_BUTTON_WIDTH`, `ProductChooserAction`).
    fn popup_options() -> PopupOptions {
        PopupOptions {
            min_width: Some(BUTTON_WIDTH),
            ..PopupOptions::default()
        }
    }

    /// The main button with more than one folder: their list below it.
    fn open_main_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.found.main.iter().map(product_row).collect();
        let position = self.main_anchor.get().bottom_left() + gpui::point(px(0.0), px(4.0));
        self.open_popup(items, position, window, cx);
    }

    fn open_popup(
        &mut self,
        items: Vec<MenuItem>,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popup(cx);
        let focus = self.focus.clone();
        self.popup = MenuPopup::open_with(
            items,
            Self::popup_options(),
            position,
            &focus,
            self.activate,
            window,
            cx,
            Self::close_popup,
        );
        cx.notify();
    }

    fn close_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.popup.take() {
            popup.close(cx);
            cx.notify();
        }
    }

    fn on_choose_folder(&mut self, a: &ChooseFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(cx);
        if let Some(path) = &a.path {
            self.import_path(std::path::PathBuf::from(path), cx);
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.import_path(path, cx));
            }
        })
        .detach();
    }

    fn import_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        match settings_import::pycharm::options::custom_folder(&path) {
            Some(folder) => self.import(folder, cx),
            None => {
                self.error = Some(unrecognized(&path.to_string_lossy()).into());
                cx.notify();
            }
        }
    }

    // ---- closing ----------------------------------------------------------------

    /// Enter: the newest folder is imported, as its button does.
    fn on_continue(&mut self, _: &Continue, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Import
            && let Some(folder) = self.folder(None)
        {
            self.import(folder, cx);
        }
    }

    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Progress {
            self.close(cx);
        }
    }

    /// Closes the window and goes on.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.close_popup(cx);
        self.finished(cx);
        let handle = self.window;
        if self.in_dialog {
            cx.defer(move |cx| ui::dialog_window::close(handle, cx));
        } else {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    /// The first start is over: what was not decided counts as skipped,
    /// and the start-up goes on.
    fn finished(&mut self, cx: &mut Context<Self>) {
        super::decide_skipped(cx);
        if let Some(done) = self.done.take() {
            cx.defer(done);
        }
    }

    /// Settings opened from the window are modal to it.
    fn on_open_settings(&mut self, a: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        let request = crate::OpenRequest {
            project_name: None,
            context: crate::PageContext::default(),
            page: a.page.clone(),
            filter: a.filter.clone(),
            activate: self.activate,
            modal: true,
        };
        crate::open(window, request, cx);
    }

    // ---- state ------------------------------------------------------------------

    fn state_json(&self, cx: &App) -> Json {
        let folder = |f: &ConfigFolder| {
            json!({
                "name": f.name,
                "label": f.label(),
                "path": f.path.to_string_lossy(),
                "used": used(f),
            })
        };
        json!({
            "page": self.page.name(),
            "title": self.title.as_ref(),
            "main": self.found.main.iter().map(folder).collect::<Vec<_>>(),
            "other": self.found.other.iter().map(folder).collect::<Vec<_>>(),
            "error": self.error.as_ref().map(|e| e.to_string()),
            "importing": self.importing.as_ref().map(|f| f.name.clone()),
            "progress": [self.progress.0, self.progress.1],
            "popup": self.popup.as_ref().map(|p| p.menu.read(cx).state_json()),
            "decided": crate::store::get::<ImportSettings>(cx).source,
        })
    }

    // ---- drawing ----------------------------------------------------------------

    fn render_import(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let h1 = theme.ui_size() + px(H1_PLUS);
        let hover_border: Hsla = rgb(if theme.is_dark() {
            HOVER_BORDER_DARK
        } else {
            HOVER_BORDER_LIGHT
        })
        .into();
        let radius = ui::form::field_radius(cx);
        let border = theme.color(UiRole::BorderInput);
        let link = theme.color(UiRole::TextLink);
        let hover = theme.color(UiRole::Hover);
        let mut buttons: Vec<AnyElement> = Vec::new();
        let main = self.found.main.clone();
        if !main.is_empty() {
            let several = main.len() > 1;
            let label = if several {
                REFERENCE.to_owned()
            } else {
                main[0].label()
            };
            let anchor = self.main_anchor.clone();
            buttons.push(
                div()
                    .id("first-run-main")
                    .debug_selector(|| "first-run-main".into())
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .w(px(BUTTON_WIDTH))
                    .h(px(BUTTON_HEIGHT))
                    .my(px(BUTTON_GAP))
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .hover(move |s| s.border_color(hover_border))
                    .child(label)
                    .when(several, |d| d.child(Icon::new(IconName::CHEVRON_DOWN)))
                    .child(anchor_canvas(anchor))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if several {
                            this.open_main_list(window, cx);
                        } else {
                            window.dispatch_action(Box::new(ImportFrom { folder: None }), cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        buttons.push(
            div()
                .id("first-run-skip")
                .debug_selector(|| "first-run-skip".into())
                .flex()
                .items_center()
                .justify_center()
                .w(px(BUTTON_WIDTH))
                .h(px(BUTTON_HEIGHT))
                .my(px(BUTTON_GAP))
                .rounded(radius)
                .text_color(link)
                .hover(move |s| s.bg(hover))
                .child(SKIP_IMPORT)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(SkipImport), cx))
                .into_any_element(),
        );
        let anchor = self.other_anchor.clone();
        let other = ui::link::link("first-run-other-options", OTHER_OPTIONS, cx)
            .debug_selector(|| "first-run-other-options".into())
            .relative()
            .gap(px(0.0))
            .child(Icon::new(IconName::CHEVRON_DOWN))
            .child(anchor_canvas(anchor))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OtherOptions), cx));
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .text_size(h1)
                                    .font_weight(FontWeight::BOLD)
                                    .mb(px(TITLE_GAP - BUTTON_GAP))
                                    .child(IMPORT_TITLE),
                            )
                            .children(buttons),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(SOUTH_HEIGHT))
                    .pl(px(20.0))
                    .pr(px(7.0))
                    .pb(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .child(other),
            )
            .into_any_element()
    }

    fn render_progress(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let h1 = theme.ui_size() + px(H1_PLUS);
        let (done, all) = self.progress;
        let fraction = if all == 0 {
            0.0
        } else {
            done as f32 / all as f32
        };
        let message = self
            .importing
            .as_ref()
            .map(|f| format!("Importing from {}\u{2026}", f.label()));
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(h1)
                    .font_weight(FontWeight::BOLD)
                    .child(IMPORTING),
            )
            .children(message.map(|m| div().mt(px(8.0)).child(m)))
            .child(
                div()
                    .mt(px(40.0))
                    .w(px(BUTTON_WIDTH))
                    .h(px(4.0))
                    .rounded_full()
                    .bg(theme.color(UiRole::ProgressTrack))
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .w(px(BUTTON_WIDTH * fraction))
                            .bg(theme.color(UiRole::Progress)),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for FirstRunWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FirstRunWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let surface = theme.color(UiRole::Surface);
        let danger_bg = theme.color(UiRole::DangerBg);
        let page = match self.page {
            Page::Import => self.render_import(cx),
            Page::Progress => self.render_progress(cx),
        };
        let error = self.error.clone().map(|e| {
            div()
                .debug_selector(|| "first-run-error".into())
                .absolute()
                .top(px(ui::dialog_window::title_height(true)))
                .left_0()
                .right_0()
                .px(px(16.0))
                .py(px(10.0))
                .bg(danger_bg)
                .child(e)
        });
        ui::components::root(cx)
            .id("first-run")
            .debug_selector(|| "first-run".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_import))
            .on_action(cx.listener(Self::on_skip))
            .on_action(cx.listener(Self::on_other_options))
            .on_action(cx.listener(Self::on_choose_folder))
            .on_action(cx.listener(Self::on_continue))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_open_settings))
            .relative()
            .flex()
            .flex_col()
            .bg(surface)
            .child(glow(cx))
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(page)
            .children(error)
            .children(ui::dialog_window::blocker(window, cx))
            .child(ui::pointer::frame())
    }
}

/// Our own glow behind the page (the reference shows a blurred picture of
/// its own there): green at the top left, teal at the top, olive at the
/// right.
fn glow(cx: &App) -> gpui::Div {
    let alpha = if cx.theme().is_dark() { 1.0 } else { 0.25 };
    let blob = |x: f32, y: f32, size: f32, color: u32| {
        div()
            .absolute()
            .left(gpui::relative(x))
            .top(gpui::relative(y))
            .ml(px(-size / 2.0))
            .mt(px(-size / 2.0))
            .size(px(size))
            .rounded_full()
            .shadow(vec![gpui::BoxShadow {
                color: Hsla::from(rgb(color)).opacity(alpha),
                offset: gpui::point(px(0.0), px(0.0)),
                blur_radius: px(size * 0.5),
                spread_radius: px(0.0),
            }])
    };
    div()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .child(blob(0.08, 0.14, 220.0, 0x266247))
        .child(blob(0.42, -0.02, 240.0, 0x2c7a82))
        .child(blob(1.0, 0.62, 220.0, 0x504c21))
}

/// Keeps an element's bounds for a popup below it.
fn anchor_canvas(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// `import.chooser.error.unrecognized` (`BootstrapBundle`).
fn unrecognized(path: &str) -> String {
    format!("{path} does not appear to be this IDE's settings or installation directory")
}

/// A folder's row in the lists of the main button and Other Options
/// (`ProductChooserRenderer`): the product's icon (our settings icon in
/// place of the reference's logo) at the top, the name over when it was
/// used.
fn product_row(folder: &ConfigFolder) -> MenuItem {
    MenuItem::action(
        folder.label(),
        ImportFrom {
            folder: Some(folder.name.clone()),
        },
    )
    .icon(IconName::SETTINGS)
    .icon_size(ROW_ICON)
    .comment(used(folder))
}

/// When a folder was used last, as the reference says it
/// (`ActionsDataProvider.toRelativeFormat` with `used`).
pub fn used(folder: &ConfigFolder) -> String {
    let days = std::time::SystemTime::now()
        .duration_since(folder.last_used)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    used_days_ago(days)
}

fn used_days_ago(days: u64) -> String {
    match days {
        0 => "Used today".to_owned(),
        1 => "Used yesterday".to_owned(),
        2..=6 => format!("Used {days} days ago"),
        7..=29 if days / 7 == 1 => "Used a week ago".to_owned(),
        7..=29 => format!("Used {} weeks ago", days / 7),
        30..=364 if days / 30 == 1 => "Used a month ago".to_owned(),
        30..=364 => format!("Used {} months ago", days / 30),
        _ if days / 365 == 1 => "Used a year ago".to_owned(),
        _ => format!("Used {} years ago", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_use_reads_as_the_reference_says_it() {
        assert_eq!(used_days_ago(0), "Used today");
        assert_eq!(used_days_ago(1), "Used yesterday");
        assert_eq!(used_days_ago(3), "Used 3 days ago");
        assert_eq!(used_days_ago(8), "Used a week ago");
        assert_eq!(used_days_ago(20), "Used 2 weeks ago");
        assert_eq!(used_days_ago(40), "Used a month ago");
        assert_eq!(used_days_ago(100), "Used 3 months ago");
        assert_eq!(used_days_ago(400), "Used a year ago");
        assert_eq!(used_days_ago(800), "Used 2 years ago");
    }
}
