//! The first-run window's pages, as the reference's onboarding dialog lays
//! them out (IntelliJ Community, Apache-2.0: `ProductChooserPage`,
//! `ImportProgressPage`, `ThemeChooserPage`, `KeymapChooserPage`,
//! `WizardPagePane`, `SchemePane`, `KeymapPane`, `RoundedPanel`,
//! `UiUtils`):
//!
//! - Import Settings: the title (`JBFont.h1`) 26 above a column of 280 by
//!   40 buttons 8 apart (the newest folder, then Skip Import as a link
//!   button), and Other Options at the bottom right;
//! - Importing settings: the title, "Importing from …", a 280 wide bar;
//! - Choose Theme and Choose Keymap: a line, the title (the label font
//!   plus 7, 18 above and 17 below) with Dark and Light at the right of
//!   Choose Theme, the cards side by side 9 apart (the chosen one twice as
//!   wide, a 2 px blue outline, 1 px in blue under the mouse, else 1 px
//!   in the border colour, corners of 10), a line and the
//!   buttons (Back, Continue) in a bar 57 high, 20 from the right edge.
//!
//! A theme's card shows a few lines of code in its colours (the reference
//! shows a picture of its editor); a keymap's card shows the sample
//! shortcuts in key caps, with their names on the chosen card.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, FocusHandle, Focusable, FontWeight, Hsla,
    IntoElement, PathPromptOptions, Pixels, Render, SharedString, Task, Window, canvas, div,
    prelude::*, px, rgb,
};
use serde_json::{Value as Json, json};
use settings::ImportSettings;
use settings_import::pycharm::options::{ConfigFolder, Found};
use themes::{TextRole, ThemeKind, UiRole};
use ui::form::{BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::icons::{Icon, IconName};
use ui::menu::MenuItem;
use ui::theme::ThemeExt;

use super::import::{self, Applied};
use super::{
    Back, Cancel, ChooseFolder, Continue, Done, ImportFrom, KEY_CONTEXT, OtherOptions,
    SelectKeymap, SelectNext, SelectPrevious, SelectTheme, SelectThemeKind, SkipImport, Start,
};
use crate::commands::OpenSettings;
use crate::{store, theme_switch};

/// Strings of the reference (`ImportSettingsBundle`, `BootstrapBundle`).
pub const IMPORT_TITLE: &str = "Import Settings";
pub const SKIP_IMPORT: &str = "Skip Import";
pub const OTHER_OPTIONS: &str = "Other Options";
pub const LOCAL_SETTINGS: &str = "Local Settings";
pub const CUSTOM_DIR: &str = "Config or Installation Directory";
pub const IMPORTING: &str = "Importing settings\u{2026}";
pub const THEME_TITLE: &str = "Choose Theme";
pub const KEYMAP_TITLE: &str = "Choose Keymap";
pub const DARK: &str = "Dark";
pub const LIGHT: &str = "Light";
pub const BACK: &str = "Back";
pub const CONTINUE: &str = "Continue";
/// The products of the main choices under one button.
pub const REFERENCE: &str = "Reference IDE";

/// The commands whose shortcuts the keymap cards show.
pub const SAMPLE_SHORTCUTS: [(&str, &str); 5] = [
    ("Search Everywhere", "search-everywhere.open"),
    ("Find Action", "search-everywhere.go-to-action"),
    ("Go to File", "search-everywhere.go-to-file"),
    ("Find in Files", "find-in-files.open"),
    ("Run", "run.run"),
];

/// Metrics (`UiUtils`, `ProductChooserPage`, `WizardPagePane`).
const BUTTON_WIDTH: f32 = 280.0;
const BUTTON_HEIGHT: f32 = 40.0;
const BUTTON_GAP: f32 = 4.0;
const TITLE_GAP: f32 = 26.0;
const SOUTH_HEIGHT: f32 = 47.0;
const BAR_HEIGHT: f32 = 57.0;
const BAR_RIGHT: f32 = 20.0;
const CARD_SIDE: f32 = 19.0;
const CARD_BOTTOM: f32 = 13.0;
const CARD_GAP: f32 = 9.0;
const CARD_RADIUS: f32 = 10.0;
const HEADER_TOP: f32 = 18.0;
const HEADER_BOTTOM: f32 = 17.0;
/// `JBFont.h1()`: the label font plus 9, bold; `UiUtils.HEADER_FONT`:
/// plus 7.
const H1_PLUS: f32 = 9.0;
const HEADER_PLUS: f32 = 7.0;
/// `OnboardingDialogButtons.BUTTON_HOVER_BORDER_COLOR`.
const HOVER_BORDER_LIGHT: u32 = 0xa8adbd;
const HOVER_BORDER_DARK: u32 = 0x6f737a;
/// `RoundedPanel.SELECTED_BORDER_COLOR` and `BORDER_COLOR`.
const CARD_SELECTED: u32 = 0x3574f0;
const CARD_BORDER_LIGHT: u32 = 0xd3d5db;
const CARD_BORDER_DARK: u32 = 0x43454a;
/// `KeymapPane.KEY_BACKGROUND`.
const KEY_LIGHT: u32 = 0xdfe1e5;
const KEY_DARK: u32 = 0x4e5157;
/// Key caps are at most this wide (`KeymapPane`: 85).
const KEY_MAX: f32 = 85.0;
/// The code a theme's card shows.
const PREVIEW: &[&[(&str, TextRole)]] = &[
    &[
        ("class ", TextRole::Keyword),
        ("Greeter", TextRole::ClassDecl),
        (":", TextRole::Punctuation),
    ],
    &[
        ("    ", TextRole::Variable),
        ("\"\"\"Say hello.\"\"\"", TextRole::Docstring),
    ],
    &[
        ("    def ", TextRole::Keyword),
        ("greet", TextRole::FunctionDecl),
        ("(", TextRole::Punctuation),
        ("self", TextRole::SelfParameter),
        (", ", TextRole::Punctuation),
        ("name", TextRole::Parameter),
        ("):", TextRole::Punctuation),
    ],
    &[("        # one line", TextRole::Comment)],
    &[
        ("        return ", TextRole::Keyword),
        ("f\"Hi, ", TextRole::String),
        ("{name}", TextRole::FstringExpression),
        ("!\"", TextRole::String),
        (" * ", TextRole::Operator),
        ("2", TextRole::Number),
    ],
];

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
    Theme,
    Keymap,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Import => "import",
            Page::Progress => "progress",
            Page::Theme => "theme",
            Page::Keymap => "keymap",
        }
    }
}

/// A theme's card: its id, name and the colours its preview needs.
#[derive(Clone)]
struct ThemeCard {
    id: String,
    name: SharedString,
    background: Hsla,
    foreground: Hsla,
    gutter: Hsla,
    line_number: Hsla,
    text: Vec<Hsla>,
}

pub struct FirstRunWindow {
    title: SharedString,
    found: Found,
    page: Page,
    /// The import page was offered: Choose Theme has Back.
    offered_import: bool,
    /// The folder being imported or imported last.
    importing: Option<ConfigFolder>,
    /// Importers done and all of them.
    progress: (usize, usize),
    /// An error over the import page (a folder that is not a
    /// configuration folder).
    error: Option<SharedString>,
    /// Choose Theme shows the dark themes.
    dark: bool,
    /// The theme last chosen of each kind (dark, light).
    chosen: [Option<String>; 2],
    /// Resolved themes of the cards, by id.
    cards: HashMap<String, ThemeCard>,
    /// The sample shortcuts of the keymaps' cards, by keymap.
    shortcuts: HashMap<String, Vec<Option<String>>>,
    hovered: Option<usize>,
    popup: Option<MenuPopup>,
    other_anchor: Rc<Cell<Bounds<Pixels>>>,
    main_anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
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
        let page = if found.is_empty() {
            Page::Theme
        } else {
            Page::Import
        };
        // The window closed by its title bar goes on as Continue would.
        if !in_dialog {
            window.on_window_should_close(cx, |window, cx| {
                if let Some(view) = window.root::<FirstRunWindow>().flatten() {
                    view.update(cx, |view, cx| view.finished(cx));
                }
                true
            });
        }
        let dark = cx.theme().is_dark();
        Self {
            title: start.title,
            offered_import: page == Page::Import,
            found,
            page,
            importing: None,
            progress: (0, 0),
            error: None,
            dark,
            chosen: [None, None],
            cards: HashMap::new(),
            shortcuts: HashMap::new(),
            hovered: None,
            popup: None,
            other_anchor: Rc::default(),
            main_anchor: Rc::default(),
            focus,
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
        self.hovered = None;
        if page == Page::Theme {
            self.dark = cx.theme().is_dark();
        }
        // Keymaps may have come in with an import.
        self.shortcuts.clear();
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

    /// Imports `folder` in the background, then shows Choose Theme.
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
                this.go(Page::Theme, cx);
            });
        }));
    }

    fn on_skip(&mut self, _: &SkipImport, _: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Import {
            return;
        }
        super::decide_skipped(cx);
        self.go(Page::Theme, cx);
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
                    MenuItem::action(
                        folder.label(),
                        ImportFrom {
                            folder: Some(folder.name.clone()),
                        },
                    )
                    .secondary(used(folder))
                    .description(folder.path.to_string_lossy().into_owned()),
                );
            }
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::action(
            format!("{CUSTOM_DIR}\u{2026}"),
            ChooseFolder { path: None },
        ));
        let position = self.other_anchor.get().bottom_left() + gpui::point(px(0.0), px(4.0));
        self.open_popup(items, position, window, cx);
    }

    /// The main button with more than one folder: their list below it.
    fn open_main_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self
            .found
            .main
            .iter()
            .map(|folder| {
                MenuItem::action(
                    folder.label(),
                    ImportFrom {
                        folder: Some(folder.name.clone()),
                    },
                )
                .secondary(used(folder))
            })
            .collect();
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
        self.popup = MenuPopup::open(
            items,
            None,
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

    // ---- going between pages ----------------------------------------------------

    fn on_back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Keymap => self.go(Page::Theme, cx),
            Page::Theme if self.offered_import => self.go(Page::Import, cx),
            _ => {}
        }
    }

    fn on_continue(&mut self, _: &Continue, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Import => {
                if let Some(folder) = self.folder(None) {
                    self.import(folder, cx);
                }
            }
            Page::Progress => {}
            Page::Theme => self.go(Page::Keymap, cx),
            Page::Keymap => self.close(window, cx),
        }
    }

    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Progress {
            self.close(window, cx);
        }
    }

    /// Closes the window and goes on.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(cx);
        self.finished(cx);
        if self.in_dialog {
            let handle = window.window_handle();
            cx.defer(move |cx| ui::dialog_window::close(handle, cx));
        } else {
            window.remove_window();
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

    // ---- themes -----------------------------------------------------------------

    /// The themes of the kind shown, in the catalog's order.
    fn theme_ids(&self, cx: &App) -> Vec<String> {
        let kind = if self.dark {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        };
        theme_switch::catalog(cx)
            .themes()
            .filter(|e| e.kind == kind)
            .map(|e| e.id.clone())
            .collect()
    }

    fn card(&mut self, id: &str, cx: &App) -> Option<ThemeCard> {
        if let Some(card) = self.cards.get(id) {
            return Some(card.clone());
        }
        let catalog = theme_switch::catalog(cx);
        let entry = catalog.get(id)?;
        let theme = catalog.resolve(id).ok()?;
        let color = ui::theme::color;
        let card = ThemeCard {
            id: id.to_owned(),
            name: entry.name.clone().into(),
            background: color(theme.ui[UiRole::EditorBackground]),
            foreground: color(theme.ui[UiRole::EditorText]),
            gutter: color(theme.ui[UiRole::Gutter]),
            line_number: color(theme.ui[UiRole::LineNumber]),
            text: PREVIEW
                .iter()
                .flat_map(|line| line.iter().map(|(_, role)| color(theme.text_color(*role))))
                .collect(),
        };
        self.cards.insert(id.to_owned(), card.clone());
        Some(card)
    }

    fn select_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        let kind = theme_switch::catalog(cx).get(id).map(|e| e.kind);
        let Some(kind) = kind else {
            return;
        };
        if let Err(e) = theme_switch::switch(id, cx) {
            eprintln!("first-run: {e}");
            return;
        }
        self.dark = kind == ThemeKind::Dark;
        self.chosen[usize::from(!self.dark)] = Some(id.to_owned());
        cx.notify();
    }

    fn on_select_theme(&mut self, a: &SelectTheme, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Theme {
            self.select_theme(&a.id, cx);
        }
    }

    fn on_theme_kind(&mut self, a: &SelectThemeKind, _: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Theme {
            return;
        }
        let dark = match a.kind.as_str() {
            "dark" => true,
            "light" => false,
            _ => return,
        };
        self.set_kind(dark, cx);
    }

    /// Dark or Light: the last theme chosen of that kind, else its first.
    fn set_kind(&mut self, dark: bool, cx: &mut Context<Self>) {
        if dark == self.dark && dark == cx.theme().is_dark() {
            return;
        }
        self.dark = dark;
        let ids = self.theme_ids(cx);
        let id = self.chosen[usize::from(!dark)]
            .clone()
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().cloned());
        if let Some(id) = id {
            self.select_theme(&id, cx);
        }
        cx.notify();
    }

    // ---- keymaps ----------------------------------------------------------------

    fn keymaps(cx: &App) -> Vec<String> {
        super::keymap_choice(cx)
            .map(|c| (c.keymaps)(cx))
            .unwrap_or_default()
    }

    fn current_keymap(cx: &App) -> Option<String> {
        super::keymap_choice(cx).map(|c| (c.current)(cx))
    }

    fn select_keymap(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(choice) = super::keymap_choice(cx) else {
            return;
        };
        if let Err(e) = (choice.switch)(name, cx) {
            eprintln!("first-run: {e}");
        }
        cx.notify();
    }

    fn on_select_keymap(&mut self, a: &SelectKeymap, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Keymap {
            self.select_keymap(&a.name, cx);
        }
    }

    /// The card `by` from the chosen one on Choose Theme or Choose Keymap.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let (names, current) = match self.page {
            Page::Theme => (self.theme_ids(cx), theme_switch::current_theme(cx)),
            Page::Keymap => (
                Self::keymaps(cx),
                Self::current_keymap(cx).unwrap_or_default(),
            ),
            _ => return,
        };
        let Some(at) = names.iter().position(|n| *n == current) else {
            return;
        };
        let next = at as isize + by;
        if next < 0 || next as usize >= names.len() {
            return;
        }
        let name = names[next as usize].clone();
        match self.page {
            Page::Theme => self.select_theme(&name, cx),
            _ => self.select_keymap(&name, cx),
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
        let mut state = json!({
            "page": self.page.name(),
            "title": self.title.as_ref(),
            "main": self.found.main.iter().map(folder).collect::<Vec<_>>(),
            "other": self.found.other.iter().map(folder).collect::<Vec<_>>(),
            "error": self.error.as_ref().map(|e| e.to_string()),
            "importing": self.importing.as_ref().map(|f| f.name.clone()),
            "progress": [self.progress.0, self.progress.1],
            "popup": self.popup.as_ref().map(|p| p.menu.read(cx).state_json()),
            "decided": crate::store::get::<ImportSettings>(cx).source,
            "buttons": self.buttons().iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        });
        match self.page {
            Page::Theme => {
                let current = theme_switch::current_theme(cx);
                state["kind"] = json!(if self.dark { "dark" } else { "light" });
                state["cards"] = json!(self.theme_ids(cx));
                state["chosen"] = json!(current);
            }
            Page::Keymap => {
                let current = Self::current_keymap(cx);
                let cards: Vec<Json> = Self::keymaps(cx)
                    .iter()
                    .map(|name| {
                        let keys = sample_shortcuts(name, cx);
                        let shortcuts: Vec<Json> = SAMPLE_SHORTCUTS
                            .iter()
                            .zip(keys)
                            .map(|((label, _), keys)| json!({"name": label, "keys": keys}))
                            .collect();
                        json!({"name": name, "shortcuts": shortcuts})
                    })
                    .collect();
                state["cards"] = json!(cards);
                state["chosen"] = json!(current);
            }
            _ => {}
        }
        state
    }

    /// The bar's buttons of the page: id, label, primary.
    fn buttons(&self) -> Vec<(&'static str, &'static str, bool)> {
        match self.page {
            Page::Theme if self.offered_import => {
                vec![
                    ("first-run-continue", CONTINUE, true),
                    ("first-run-back", BACK, false),
                ]
            }
            Page::Theme => vec![("first-run-continue", CONTINUE, true)],
            Page::Keymap => {
                vec![
                    ("first-run-continue", CONTINUE, true),
                    ("first-run-back", BACK, false),
                ]
            }
            _ => Vec::new(),
        }
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

    /// A card's outline over its content, in a 2 px band
    /// (`RoundedBorder`): 2 px in the selection colour when chosen, 1 px
    /// in it under the mouse, else 1 px in the border colour, a pixel in
    /// (`RoundedPanel`: fixed colours, whatever the theme).
    fn outline(chosen: bool, hovered: bool, cx: &App) -> gpui::Div {
        let accent: Hsla = rgb(CARD_SELECTED).into();
        let border: Hsla = rgb(if cx.theme().is_dark() {
            CARD_BORDER_DARK
        } else {
            CARD_BORDER_LIGHT
        })
        .into();
        let ring = div().absolute().rounded(px(CARD_RADIUS));
        match (chosen, hovered) {
            (true, _) => ring.inset_0().border_2().border_color(accent),
            (false, hover) => ring
                .top(px(1.0))
                .left(px(1.0))
                .right(px(1.0))
                .bottom(px(1.0))
                .border_1()
                .border_color(if hover { accent } else { border }),
        }
    }

    fn render_cards(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut cards: Vec<AnyElement> = Vec::new();
        match self.page {
            Page::Theme => {
                let ids = self.theme_ids(cx);
                let current = theme_switch::current_theme(cx);
                let mono = cx.theme().mono_family();
                for (ix, id) in ids.iter().enumerate() {
                    let Some(card) = self.card(id, cx) else {
                        continue;
                    };
                    let chosen = *id == current;
                    let hovered = self.hovered == Some(ix);
                    cards.push(self.theme_card(ix, card, chosen, hovered, mono.clone(), cx));
                }
            }
            Page::Keymap => {
                let names = Self::keymaps(cx);
                let current = Self::current_keymap(cx).unwrap_or_default();
                for (ix, name) in names.iter().enumerate() {
                    let chosen = *name == current;
                    let hovered = self.hovered == Some(ix);
                    let keys = self
                        .shortcuts
                        .entry(name.clone())
                        .or_insert_with(|| sample_shortcuts(name, cx))
                        .clone();
                    cards.push(self.keymap_card(ix, name, keys, chosen, hovered, cx));
                }
            }
            _ => {}
        }
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_row()
            .gap(px(CARD_GAP))
            .children(cards)
            .into_any_element()
    }

    fn card_frame(
        &self,
        ix: usize,
        kind: &'static str,
        chosen: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut card = div()
            .id((kind, ix))
            .debug_selector(move || format!("first-run-{kind}-{ix}"))
            .relative()
            .h_full()
            .min_w(px(0.0))
            .flex_basis(px(0.0))
            .flex_shrink()
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over {
                    this.hovered = Some(ix);
                } else if this.hovered == Some(ix) {
                    this.hovered = None;
                }
                cx.notify();
            }));
        card.style().flex_grow = Some(if chosen { 2.0 } else { 1.0 });
        card
    }

    fn theme_card(
        &self,
        ix: usize,
        card: ThemeCard,
        chosen: bool,
        hovered: bool,
        mono: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let size = cx.theme().ui_size() - px(2.0);
        let mut colors = card.text.iter();
        let lines: Vec<AnyElement> = PREVIEW
            .iter()
            .enumerate()
            .map(|(n, line)| {
                let spans: Vec<AnyElement> = line
                    .iter()
                    .map(|(text, _)| {
                        let color = colors.next().copied().unwrap_or(card.line_number);
                        div().text_color(color).child(*text).into_any_element()
                    })
                    .collect();
                div()
                    .flex()
                    .flex_row()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_none()
                            .w(px(22.0))
                            .pr(px(8.0))
                            .flex()
                            .justify_end()
                            .bg(card.gutter)
                            .text_color(card.line_number)
                            .child(format!("{}", n + 1)),
                    )
                    .child(div().pl(px(6.0)).flex().flex_row().children(spans))
                    .into_any_element()
            })
            .collect();
        let id = card.id.clone();
        let frame = self.card_frame(ix, "theme", chosen, cx);
        frame
            .p(px(2.0))
            .child(
                div()
                    .size_full()
                    .rounded(px(CARD_RADIUS - 2.0))
                    .overflow_hidden()
                    .bg(card.background)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .w_full()
                            .pt(px(9.0))
                            .pb(px(5.0))
                            .flex()
                            .justify_center()
                            .text_color(card.foreground)
                            .font_weight(FontWeight::BOLD)
                            .whitespace_nowrap()
                            .child(card.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .pt(px(4.0))
                            .font_family(mono)
                            .text_size(size)
                            .line_height(px(18.0))
                            .children(lines),
                    ),
            )
            .child(Self::outline(chosen, hovered, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select_theme(&id, cx)))
            .into_any_element()
    }

    fn keymap_card(
        &self,
        ix: usize,
        name: &str,
        keys: Vec<Option<String>>,
        chosen: bool,
        hovered: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let key_ring: Hsla = rgb(if theme.is_dark() { KEY_DARK } else { KEY_LIGHT }).into();
        let surface = theme.color(UiRole::SurfaceAlt);
        let small = theme.ui_size() - px(1.0);
        let rows: Vec<AnyElement> = SAMPLE_SHORTCUTS
            .iter()
            .zip(keys)
            .map(|((label, _), keys)| {
                let keys = keys.unwrap_or_else(|| "\u{2013}".to_owned());
                let cap = div()
                    .flex_none()
                    .max_w(px(KEY_MAX))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .px(px(4.0))
                    .rounded(px(4.0))
                    .border_2()
                    .border_color(key_ring)
                    .text_size(small)
                    .child(keys);
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(8.0))
                    .mb(px(14.0))
                    .when(chosen, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(*label),
                        )
                    })
                    .when(!chosen, |d| d.justify_center())
                    .child(cap)
                    .into_any_element()
            })
            .collect();
        let target = name.to_owned();
        let frame = self.card_frame(ix, "keymap", chosen, cx);
        frame
            .p(px(2.0))
            .child(
                div()
                    .size_full()
                    .rounded(px(CARD_RADIUS - 2.0))
                    .overflow_hidden()
                    .bg(surface)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .py(px(13.0))
                            .flex()
                            .justify_center()
                            .font_weight(FontWeight::BOLD)
                            .whitespace_nowrap()
                            .child(name.to_owned()),
                    )
                    .children(rows),
            )
            .child(Self::outline(chosen, hovered, cx))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.select_keymap(&target, cx)),
            )
            .into_any_element()
    }

    fn render_wizard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let header = theme.ui_size() + px(HEADER_PLUS);
        let line = theme.color(UiRole::Border);
        let title = if self.page == Page::Theme {
            THEME_TITLE
        } else {
            KEYMAP_TITLE
        };
        let kinds = (self.page == Page::Theme).then(|| {
            let choices: [SharedString; 2] = [DARK.into(), LIGHT.into()];
            let selected = Some(usize::from(!self.dark));
            ui::form::segmented(
                "first-run-kind",
                &choices,
                selected,
                |ix, window, cx| {
                    let kind = if ix == 0 { "dark" } else { "light" };
                    window.dispatch_action(
                        Box::new(SelectThemeKind {
                            kind: kind.to_owned(),
                        }),
                        cx,
                    )
                },
                cx,
            )
        });
        let cards = self.render_cards(cx);
        let mut bar = BottomBar::new().metrics(BAR_HEIGHT, BAR_RIGHT);
        for (id, label, primary) in self.buttons() {
            let action: Box<dyn gpui::Action> = if primary {
                Box::new(Continue)
            } else {
                Box::new(Back)
            };
            let role = if primary {
                ButtonRole::Ok
            } else {
                ButtonRole::Other
            };
            bar = bar.button(BarButton::new(id, label, role, action).primary(primary));
        }
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(line)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .px(px(CARD_SIDE))
                    .pb(px(CARD_BOTTOM))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .pt(px(HEADER_TOP))
                            .pb(px(HEADER_BOTTOM))
                            .child(div().flex_1().text_size(header).child(title))
                            .children(kinds),
                    )
                    .child(cards),
            )
            .child(bar.render(cx))
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
            Page::Theme | Page::Keymap => self.render_wizard(cx),
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
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_continue))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_select_theme))
            .on_action(cx.listener(Self::on_theme_kind))
            .on_action(cx.listener(Self::on_select_keymap))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .relative()
            .flex()
            .flex_col()
            .bg(surface)
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(page)
            .children(error)
            .children(ui::dialog_window::blocker(window, cx))
            .child(ui::pointer::frame())
    }
}

/// The sample shortcuts of a keymap (`None`: not assigned).
fn sample_shortcuts(keymap: &str, cx: &App) -> Vec<Option<String>> {
    let commands: Vec<&str> = SAMPLE_SHORTCUTS.iter().map(|(_, c)| *c).collect();
    match super::keymap_choice(cx) {
        Some(choice) => (choice.shortcuts)(keymap, &commands, cx),
        None => vec![None; commands.len()],
    }
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
