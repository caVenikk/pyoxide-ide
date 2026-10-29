//! The workbench's application settings (ADR 0017): the sections it reads
//! from the store, the pages it adds to the Settings window, the command
//! that opens the window from a main window, the main toolbar's gear menu
//! and the switchers of a main window (the Theme and Keymap popups, from
//! the gear menu and Quick Switch Scheme).

mod first_run;
mod keymap;
mod system;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, Entity, IntoElement, MouseButton,
    Pixels, Subscription, Window, anchored, canvas, div, point, prelude::*, px,
};
use settings_ui::commands::{OpenSettings, QuickSwitch, SwitchTheme};
use settings_ui::pages::{appearance, font, general};
use settings_ui::theme_switch;
use settings_ui::{OpenRequest, PageContext, PageSpec, Scope};
use ui::components::{Tooltip, icon_button};
use ui::deferred;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::notification::Notification;

use crate::keymap::active::{self as keymaps, SwitchKeymap};
use crate::workbench::Workbench;

/// The gear's tooltip (`settings.entry.point.tooltip`).
pub const GEAR_TOOLTIP: &str = "IDE and Project Settings";

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// The main toolbar's gear menu.
    ShowGearMenu => "workbench.settings-menu", "IDE and Project Settings",
        "Shows the main toolbar's gear menu below the gear: Settings\u{2026} and the switchers that join it.";
}

pub use ::settings::{DataViewSettings, SaveSettings, SyncSettings};

/// Id of the Appearance & Behavior group.
pub const APPEARANCE_BEHAVIOR: &str = "appearance-behavior";
/// Id of the Editor group.
pub const EDITOR: &str = "editor";
/// Id of the Tools group (modules put their tool pages under it).
pub const TOOLS: &str = "tools";
/// Id of the Keymap page.
pub const KEYMAP_PAGE: &str = keymap::ID;

/// The platform's pages, in the reference's places (`PlatformExtensions.xml`
/// group weights).
fn pages() -> Vec<PageSpec> {
    vec![
        PageSpec {
            weight: 100,
            description: Some(
                "Customize IDE appearance and behavior: change themes and font size, tune the keymap, and configure system settings.",
            ),
            ..PageSpec::group(APPEARANCE_BEHAVIOR, None, "Appearance & Behavior")
        },
        PageSpec {
            id: appearance::ID,
            parent: Some(APPEARANCE_BEHAVIOR),
            title: appearance::TITLE,
            weight: 150,
            scope: Scope::Application,
            search_terms: appearance::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(appearance::AppearancePage::new)),
        },
        PageSpec {
            id: system::ID,
            parent: Some(APPEARANCE_BEHAVIOR),
            title: system::TITLE,
            weight: 120,
            scope: Scope::Application,
            search_terms: system::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(system::SystemPage::new)),
        },
        PageSpec {
            weight: 60,
            description: Some(
                "Personalize source code appearance by changing fonts, highlighting styles, indents, etc. Customize the Editor from line numbers, caret placement and tabs to source code inspections, setting up templates and file encodings.",
            ),
            ..PageSpec::group(EDITOR, None, "Editor")
        },
        PageSpec {
            id: general::ID,
            parent: Some(EDITOR),
            title: general::TITLE,
            weight: 190,
            scope: Scope::Application,
            search_terms: general::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(general::GeneralPage::new)),
        },
        PageSpec {
            id: font::ID,
            parent: Some(EDITOR),
            title: font::TITLE,
            weight: 188,
            scope: Scope::Application,
            search_terms: font::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(font::FontPage::new)),
        },
        // After Editor, Plugins, Version Control and Languages & Frameworks
        // (`PlatformExtensions.xml`: the group weights go down from Keymap).
        PageSpec {
            weight: 20,
            description: Some(
                "Configure the tools built into the IDE: the database tools, the terminal and others.",
            ),
            ..PageSpec::group(TOOLS, None, "Tools")
        },
        PageSpec {
            id: keymap::ID,
            parent: None,
            title: keymap::TITLE,
            weight: keymap::WEIGHT,
            scope: Scope::Application,
            search_terms: keymap::search_terms(),
            description: None,
            create: Some(settings_ui::page::create(keymap::KeymapPage::new)),
        },
    ]
}

/// Registers the settings commands, the workbench's sections and pages and
/// the product's pages; every main window follows changes of its sections.
pub fn init(cx: &mut App, product_pages: &[PageSpec]) {
    settings_ui::init(cx);
    ui::command::register(cx, COMMANDS);
    settings_ui::store::register::<SaveSettings>(cx);
    settings_ui::store::register::<SyncSettings>(cx);
    settings_ui::store::register::<::settings::SoftWrapSettings>(cx);
    settings_ui::store::register::<DataViewSettings>(cx);
    settings_ui::page::register(cx, pages());
    settings_ui::page::register(cx, product_pages.iter().cloned());
    // After the current update: the change may come from a main window
    // (`settings.set` there).
    settings_ui::store::observe(cx, <SaveSettings as ::settings::Section>::KEY, |cx| {
        cx.defer(follow_files_settings)
    });
    settings_ui::store::observe(
        cx,
        <::settings::SoftWrapSettings as ::settings::Section>::KEY,
        |cx| cx.defer(crate::editor_settings::follow),
    );
    settings_ui::store::observe(cx, <DataViewSettings as ::settings::Section>::KEY, |cx| {
        cx.defer(follow_data_view_settings)
    });
    // Theme and keymap imports report in a main window.
    theme_switch::set_notify(cx, Rc::new(notify_main_window));
    keymaps::set_notify(cx, Rc::new(notify_main_window));
    first_run::init(cx);
}

/// How the first-run window opens for a product (`settings_ui::first_run`).
pub fn first_run_start(title: &str) -> settings_ui::first_run::Start {
    first_run::start(title)
}

/// Shows a balloon in the active main window (the one under an active
/// Settings window), else in the first.
pub(crate) fn notify_main_window(notification: Notification, cx: &mut App) {
    let active = cx.active_window();
    let target = active
        .and_then(|w| w.downcast::<Workbench>())
        .or_else(|| {
            active
                .and_then(|w| ui::dialog_window::parent_of(w, cx))
                .and_then(|w| w.downcast::<Workbench>())
        })
        .or_else(|| cx.windows().iter().find_map(|w| w.downcast::<Workbench>()));
    match target {
        Some(window) => {
            let _ = window.update(cx, |w, _, cx| w.notify(notification, cx));
        }
        None => eprintln!("{}: {}", notification.title, notification.content),
    }
}

/// Whether the editors show the values of variables after their lines
/// (Show Variable Values in Editor).
pub fn show_values_inline(cx: &App) -> bool {
    settings_ui::store::get::<DataViewSettings>(cx).show_values_inline
}

/// Stores Show Variable Values in Editor; every main window follows.
pub(crate) fn set_show_values_inline(on: bool, cx: &mut App) {
    settings_ui::store::set(
        cx,
        &DataViewSettings {
            show_values_inline: on,
        },
    );
}

/// Every main window's debugger takes the data view settings of the store
/// (the check mark of Show Variable Values in Editor and the values in
/// the editors).
fn follow_data_view_settings(cx: &mut App) {
    let on = show_values_inline(cx);
    for window in cx.windows() {
        let Some(handle) = window.downcast::<Workbench>() else {
            continue;
        };
        let _ = handle.update(cx, |w, _, cx| {
            w.debugger_view()
                .update(cx, |v, cx| v.set_inline_values_shown(on, cx));
        });
    }
}

/// Every main window takes the autosave and sync settings of the store.
fn follow_files_settings(cx: &mut App) {
    let save: SaveSettings = settings_ui::store::get(cx);
    let sync: SyncSettings = settings_ui::store::get(cx);
    for window in cx.windows() {
        let Some(handle) = window.downcast::<Workbench>() else {
            continue;
        };
        let _ = handle.update(cx, |w, _, _| {
            w.set_save_settings(save);
            w.set_sync_settings(SyncSettings {
                watch: w.sync_settings().watch,
                ..sync
            });
        });
    }
}

impl Workbench {
    /// `settings.open`: the Settings window over this window.
    pub(crate) fn open_settings(
        &mut self,
        a: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.project_root(cx);
        let request = OpenRequest {
            project_name: project
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            context: PageContext { project },
            page: a.page.clone(),
            filter: a.filter.clone(),
            activate: !crate::env::background(),
            modal: false,
        };
        if settings_ui::open(window, request, cx).is_none() {
            self.show_notice("The Settings window could not open".into(), cx);
        }
    }

    /// The main toolbar's gear (`SettingsEntryPoint`); its menu opens
    /// below it.
    pub(crate) fn render_gear(&self, size: Pixels, cx: &App) -> impl IntoElement {
        let anchor = self.gear.anchor.clone();
        icon_button("settings", IconName::SETTINGS, size, px(20.0), false, cx)
            .debug_selector(|| "toolbar-settings".into())
            .relative()
            .tooltip(Tooltip::text(GEAR_TOOLTIP, None))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .on_click(|_: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(ShowGearMenu), cx)
            })
    }

    /// `workbench.settings-menu`: the gear's menu below it.
    pub(crate) fn show_gear_menu(
        &mut self,
        _: &ShowGearMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.gear.anchor.get().bottom_left();
        self.open_run_menu(gear_menu(), position, window, cx);
    }

    /// `settings.switch-theme`: a theme by id at once; without one, the
    /// Theme popup (`QuickChangeLookAndFeel`) with the current theme
    /// selected.
    pub(crate) fn switch_theme(
        &mut self,
        a: &SwitchTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &a.id {
            Some(id) => {
                if let Err(e) = theme_switch::switch(id, cx) {
                    self.show_notice(e.into(), cx);
                }
            }
            None => {
                let items = theme_switch::theme_items(cx);
                let selected = theme_switch::current_row(&items, cx);
                let menu = self.open_switch_popup(theme_switch::THEME, items, selected, window, cx);
                // The selection previews the theme it rests on.
                theme_switch::watch(&menu, cx);
            }
        }
    }

    /// `settings.switch-keymap`: a keymap by name at once; without one,
    /// the Keymap popup (`QuickChangeKeymapAction`) with the current
    /// keymap selected.
    pub(crate) fn switch_keymap(
        &mut self,
        a: &SwitchKeymap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &a.name {
            Some(name) => {
                if let Err(e) = keymaps::switch(name, cx) {
                    self.show_notice(e.into(), cx);
                }
            }
            None => {
                let (items, selected) = keymaps::popup_items(cx);
                self.open_switch_popup(keymaps::KEYMAP, items, selected, window, cx);
            }
        }
    }

    /// `settings.quick-switch`: the Switch popup (`QuickChangeSchemesAction`,
    /// the `ChangeScheme` group): Theme and Keymap open their popups.
    pub(crate) fn quick_switch(
        &mut self,
        _: &QuickSwitch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = vec![
            MenuItem::action(theme_switch::THEME, SwitchTheme { id: None }),
            MenuItem::action(keymaps::KEYMAP, SwitchKeymap { name: None }),
        ];
        self.open_switch_popup(theme_switch::SWITCH, items, None, window, cx);
    }

    /// A switcher's list popup in the middle of the window
    /// (`showCenteredInCurrentWindow`) with speed search; it replaces one
    /// open before.
    fn open_switch_popup(
        &mut self,
        title: &'static str,
        items: Vec<MenuItem>,
        selected: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ContextMenu> {
        let options = PopupOptions {
            title: Some(title.into()),
            speed_search: true,
            selected,
            // The switchers number their rows (`QuickSwitchSchemeAction`).
            numbered: true,
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this.gear.switch.as_ref().is_some_and(|(m, _)| *m == menu) {
                this.gear.switch = None;
                cx.notify();
            }
        });
        self.gear.switch = Some((menu.clone(), subscription));
        cx.notify();
        menu
    }

    /// The open switcher popup, centred over the window with a backdrop
    /// that closes it on a click.
    pub(crate) fn render_switch_popup(&self, window: &Window) -> Option<AnyElement> {
        let (menu, _) = self.gear.switch.as_ref()?;
        let size = window.viewport_size();
        let backdrop = menu.clone();
        Some(
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("switch-backdrop")
                        .occlude()
                        .w(size.width)
                        .h(size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            backdrop.update(cx, |m, cx| m.dismiss(window, cx))
                        })
                        .child(menu.clone()),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The open switcher popup for `workbench.state`.
    pub(crate) fn switch_popup_json(&self, cx: &App) -> Option<serde_json::Value> {
        let (menu, _) = self.gear.switch.as_ref()?;
        Some(menu.read(cx).state_json())
    }
}

/// Where the gear menu opens, and an open switcher popup.
#[derive(Default)]
pub(crate) struct Gear {
    anchor: Rc<Cell<Bounds<Pixels>>>,
    switch: Option<(Entity<ContextMenu>, Subscription)>,
}

/// The gear's menu, in the reference's order (`SettingsEntryPointAction`
/// with `SettingsEntryPointGroup`: update actions, Run Anything…,
/// Settings…, Plugins…, Theme…, Keymap…, View Mode…, Customize Main
/// Toolbar). Only what Pyoxide has is listed. Theme… and Keymap… open
/// their popups.
pub fn gear_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::action(
            "Settings\u{2026}",
            OpenSettings {
                page: None,
                filter: None,
            },
        ),
        MenuItem::separator(),
        MenuItem::action(theme_switch::THEME_IN_GEAR, SwitchTheme { id: None }),
        MenuItem::action(keymaps::KEYMAP_IN_GEAR, SwitchKeymap { name: None }),
    ]
}
