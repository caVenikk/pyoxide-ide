//! The workbench's application settings (ADR 0017): the sections it reads
//! from the store, the pages it adds to the Settings window, the command
//! that opens the window from a main window and the main toolbar's gear
//! menu.

mod system;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Bounds, ClickEvent, Context, IntoElement, Pixels, Window, canvas, prelude::*, px};
use settings_ui::commands::OpenSettings;
use settings_ui::{OpenRequest, PageContext, PageSpec, Scope};
use ui::components::{Tooltip, icon_button};
use ui::icons::IconName;
use ui::menu::MenuItem;

use crate::workbench::Workbench;

/// The gear's tooltip (`settings.entry.point.tooltip`).
pub const GEAR_TOOLTIP: &str = "IDE and Project Settings";

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// The main toolbar's gear menu.
    ShowGearMenu => "workbench.settings-menu", "IDE and Project Settings",
        "Shows the main toolbar's gear menu below the gear: Settings\u{2026} and the switchers that join it.";
}

pub use ::settings::{SaveSettings, SyncSettings};

/// Id of the Appearance & Behavior group.
pub const APPEARANCE_BEHAVIOR: &str = "appearance-behavior";

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
            id: system::ID,
            parent: Some(APPEARANCE_BEHAVIOR),
            title: system::TITLE,
            weight: 120,
            scope: Scope::Application,
            search_terms: system::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(system::SystemPage::new)),
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
    settings_ui::page::register(cx, pages());
    settings_ui::page::register(cx, product_pages.iter().cloned());
    // After the current update: the change may come from a main window
    // (`settings.set` there).
    settings_ui::store::observe(cx, <SaveSettings as ::settings::Section>::KEY, |cx| {
        cx.defer(follow_files_settings)
    });
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
}

/// Where the gear menu opens.
#[derive(Default)]
pub(crate) struct Gear {
    anchor: Rc<Cell<Bounds<Pixels>>>,
}

/// The gear's menu, in the reference's order (`SettingsEntryPointAction`
/// with `SettingsEntryPointGroup`: update actions, Run Anything…,
/// Settings…, Plugins…, Theme…, Keymap…, View Mode…, Customize Main
/// Toolbar). Only what Pyoxide has is listed; the theme and keymap
/// switchers join with their pages.
pub fn gear_menu() -> Vec<MenuItem> {
    vec![MenuItem::action(
        "Settings\u{2026}",
        OpenSettings {
            page: None,
            filter: None,
        },
    )]
}
