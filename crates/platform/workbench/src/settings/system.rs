//! Appearance & Behavior > System Settings (`GeneralSettingsConfigurable`):
//! the Files group with autosave and the synchronization with changes made
//! outside. Only the options Pyoxide honours are shown; the reference's
//! exit confirmation, process and project options, the move to the bin
//! (Pyoxide always deletes to the bin) and safe write (Pyoxide always
//! writes through a temporary file) are left out.

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable as _, IntoElement, Render,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::{PageContext, SettingsPage};
use themes::UiRole;
use ui::components::DISABLED_ALPHA;
use ui::theme::ThemeExt;

use super::{SaveSettings, SyncSettings};

pub const ID: &str = "system";
pub const TITLE: &str = "System Settings";
pub const SEARCH_TERMS: &[&str] = &[
    FILES,
    SAVE_ON_IDLE,
    SECONDS,
    SAVE_ON_DEACTIVATION,
    SYNC,
    SYNC_ON_ACTIVATION,
    AUTOSAVE_COMMENT,
];

/// The reference's labels (IdeBundle).
const FILES: &str = "Files";
const SAVE_ON_IDLE: &str = "Save files if the IDE is idle for";
const SECONDS: &str = "seconds";
const SAVE_ON_DEACTIVATION: &str =
    "Save files when switching to a different application or a built-in terminal";
const SYNC: &str = "Sync external changes:";
const SYNC_ON_ACTIVATION: &str = "When switching to the IDE window or opening an editor tab";
const AUTOSAVE_COMMENT: &str = "Autosave cannot be disabled completely.";

/// Kotlin UI DSL spacing: the indent of a group's rows, the gap between
/// rows, the small gap (`RightGap.SMALL`, `TopGap.SMALL`) and the width of
/// a four-column number field.
const GROUP_INDENT: f32 = 20.0;
const ROW_GAP: f32 = 6.0;
const ROW_HEIGHT: f32 = 22.0;
const SMALL_GAP: f32 = 6.0;
const TOP_GAP_SMALL: f32 = 8.0;
const NUMBER_FIELD_WIDTH: f32 = 52.0;

/// The page's draft: the check boxes and the idle time's text.
pub struct SystemPage {
    save: SaveSettings,
    sync: SyncSettings,
    idle: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SystemPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let save: SaveSettings = settings_ui::store::get(cx);
        let sync: SyncSettings = settings_ui::store::get(cx);
        let idle = editor::field::new(&save.idle_seconds.to_string(), None, true, window, cx);
        let subscriptions = vec![cx.subscribe(&idle, |_, _, e: &EditorEvent, cx| {
            if matches!(e, EditorEvent::Edited(_)) {
                cx.notify();
            }
        })];
        Self {
            save,
            sync,
            idle,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn stored(cx: &App) -> (SaveSettings, SyncSettings) {
        (settings_ui::store::get(cx), settings_ui::store::get(cx))
    }

    fn idle_text(&self, cx: &App) -> String {
        self.idle.read(cx).text().trim().to_string()
    }

    /// The draft as settings; an idle time out of range is an error
    /// (`intTextField` with `SAVE_FILES_AFTER_IDLE_SEC`).
    fn draft(&self, cx: &App) -> Result<SaveSettings, String> {
        let range = SaveSettings::IDLE_SECONDS;
        let message = || {
            format!(
                "Please enter a number from {} to {}",
                range.start(),
                range.end()
            )
        };
        let seconds: u32 = self.idle_text(cx).parse().map_err(|_| message())?;
        if !range.contains(&seconds) {
            return Err(message());
        }
        Ok(SaveSettings {
            idle_seconds: seconds,
            ..self.save
        })
    }

    fn toggle(&mut self, field: &str, cx: &mut Context<Self>) {
        match field {
            "save-on-idle" => self.save.save_on_idle = !self.save.save_on_idle,
            "save-on-deactivation" => {
                self.save.save_on_deactivation = !self.save.save_on_deactivation
            }
            "sync-on-activation" => self.sync.sync_on_activation = !self.sync.sync_on_activation,
            _ => return,
        }
        cx.notify();
    }

    fn check(
        &self,
        field: &'static str,
        checked: bool,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        ui::form::check_row(
            gpui::ElementId::Name(format!("system-row:{field}").into()),
            gpui::ElementId::Name(format!("system-check:{field}").into()),
            checked,
            div().whitespace_nowrap().child(label),
            cx,
        )
        .debug_selector(move || format!("system:{field}"))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(field, cx)))
    }
}

impl SettingsPage for SystemPage {
    fn is_modified(&self, cx: &App) -> bool {
        let (save, sync) = Self::stored(cx);
        let idle_changed = self.idle_text(cx) != save.idle_seconds.to_string();
        idle_changed
            || self.save.save_on_idle != save.save_on_idle
            || self.save.save_on_deactivation != save.save_on_deactivation
            || self.sync.sync_on_activation != sync.sync_on_activation
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let save = self.draft(cx)?;
        let (_, stored_sync) = Self::stored(cx);
        settings_ui::store::set(cx, &save);
        settings_ui::store::set(
            cx,
            &SyncSettings {
                sync_on_activation: self.sync.sync_on_activation,
                ..stored_sync
            },
        );
        self.save = save;
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (save, sync) = Self::stored(cx);
        self.save = save;
        self.sync = sync;
        let text = save.idle_seconds.to_string();
        self.idle.update(cx, |e, cx| e.set_text_selected(&text, cx));
        let _ = window;
        cx.notify();
    }

    fn focus(&self, cx: &App) -> Option<FocusHandle> {
        Some(self.idle.focus_handle(cx))
    }

    fn state(&self, cx: &App) -> Json {
        json!({
            "save-on-idle": self.save.save_on_idle,
            "idle-seconds": self.idle_text(cx),
            "save-on-deactivation": self.save.save_on_deactivation,
            "sync-on-activation": self.sync.sync_on_activation,
        })
    }

    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let flag = || {
            value
                .as_bool()
                .ok_or_else(|| format!("`{field}` takes true or false"))
        };
        match field {
            "save-on-idle" => self.save.save_on_idle = flag()?,
            "save-on-deactivation" => self.save.save_on_deactivation = flag()?,
            "sync-on-activation" => self.sync.sync_on_activation = flag()?,
            "idle-seconds" => {
                let text = match value {
                    Json::String(s) => s.clone(),
                    v => v.to_string(),
                };
                self.idle.update(cx, |e, cx| e.set_text_selected(&text, cx));
            }
            _ => return Err(format!("no field `{field}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for SystemPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().color(UiRole::TextMuted);
        let error = self.draft(cx).is_err();
        let idle_enabled = self.save.save_on_idle;
        let idle_field = div()
            .flex_none()
            .w(px(NUMBER_FIELD_WIDTH))
            .when(!idle_enabled, |d| d.opacity(DISABLED_ALPHA))
            .child(editor::field::frame(&self.idle, true, error, window, cx));
        let idle_row = div()
            .flex()
            .flex_row()
            .items_center()
            .min_h(px(ROW_HEIGHT))
            .child(self.check("save-on-idle", self.save.save_on_idle, SAVE_ON_IDLE, cx))
            .child(div().w(px(SMALL_GAP)))
            .child(idle_field)
            .child(div().w(px(SMALL_GAP)))
            .child(div().whitespace_nowrap().child(SECONDS));
        let rows: Vec<AnyElement> = vec![
            idle_row.into_any_element(),
            self.check(
                "save-on-deactivation",
                self.save.save_on_deactivation,
                SAVE_ON_DEACTIVATION,
                cx,
            )
            .min_h(px(ROW_HEIGHT))
            .into_any_element(),
            div()
                .min_h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .child(SYNC)
                .into_any_element(),
            div()
                .pl(px(GROUP_INDENT))
                .child(
                    self.check(
                        "sync-on-activation",
                        self.sync.sync_on_activation,
                        SYNC_ON_ACTIVATION,
                        cx,
                    )
                    .min_h(px(ROW_HEIGHT)),
                )
                .into_any_element(),
            div()
                .mt(px(TOP_GAP_SMALL))
                .text_color(muted)
                .child(AUTOSAVE_COMMENT)
                .into_any_element(),
        ];
        div()
            .id("system-page")
            .debug_selector(|| "system-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(ui::form::group_header(FILES, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .children(rows),
            )
    }
}
