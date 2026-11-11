//! Tools > Terminal > Font Settings (`TerminalOptionsConfigurable`, the
//! group `settings.terminal.font.settings`): the terminal's font
//! (`[terminal.font]`, the reference's `terminal-font.xml`): the font and
//! its fallback, Size, Line height and Column width. Like the reference's
//! panel the page opens with the font the terminal has now: until a font is
//! chosen that is the editor font, and a field the person leaves alone stays
//! inherited when the page is applied; the terminals follow Apply at once.
//!
//! The panel's other groups (Project Settings, Application Settings) and
//! the Configure colors link are not drawn: the options of the first have
//! no page yet and there is no Console Colors page to open.

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, IntoElement, Render,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings::TerminalFontSettings;
use settings_ui::pages::family_combo::{ComboEvent, ComboSpec, FamilyCombo};
use settings_ui::{PageContext, SettingsPage};

pub const ID: &str = "terminal";
pub const TITLE: &str = "Terminal";
pub const SEARCH_TERMS: &[&str] = &[
    FONT_SETTINGS,
    FONT,
    FALLBACK,
    SIZE,
    LINE_HEIGHT,
    COLUMN_WIDTH,
];

/// The reference's labels (`TerminalBundle`).
const FONT_SETTINGS: &str = "Font Settings";
const FONT: &str = "Font:";
const FALLBACK: &str = "Fallback:";
const SIZE: &str = "Size:";
const LINE_HEIGHT: &str = "Line height:";
const COLUMN_WIDTH: &str = "Column width:";
/// The fallback row that stands for the platform's own fallback (the
/// reference's logical font `Monospaced`).
const MONOSPACED: &str = "Monospaced";

/// Kotlin UI DSL spacing: a group's indent, the gap between rows, the gap
/// between a label and its field, between the cells of a row, and the width
/// of a four-column field.
const GROUP_INDENT: f32 = 20.0;
const ROW_GAP: f32 = 6.0;
const ROW_HEIGHT: f32 = 24.0;
const LABEL_GAP: f32 = 6.0;
const CELL_GAP: f32 = 17.0;
const NUMBER_WIDTH: f32 = 52.0;
const COMBO_WIDTH: f32 = 220.0;

/// The combo boxes: monospaced families only, without groups
/// (`isMonospacedOnly`); the fallback's first row is `Monospaced`.
const FAMILY_COMBO: ComboSpec = ComboSpec {
    prefix: "terminal-font",
    name: "family",
    width: COMBO_WIDTH,
    list_width: COMBO_WIDTH,
    monospaced_only: true,
    groups: false,
    names_only: true,
    none: None,
    warn_missing: true,
};
const FALLBACK_COMBO: ComboSpec = ComboSpec {
    name: "fallback",
    none: Some(MONOSPACED),
    warn_missing: false,
    ..FAMILY_COMBO
};

/// The keys of `settings.edit`.
mod field {
    pub const FAMILY: &str = "family";
    pub const FALLBACK: &str = "fallback";
    pub const SIZE: &str = "size";
    pub const LINE_HEIGHT: &str = "line-height";
    pub const COLUMN_WIDTH: &str = "column-width";
    pub const LIST: &str = "list";
    pub const CHOOSE: &str = "choose";
}

/// The font the terminal has when nothing is chosen: the editor font (or the
/// theme's terminal font), as the terminal view lays it out.
#[derive(Debug, Clone, PartialEq)]
struct Inherited {
    family: String,
    size: f32,
}

impl Inherited {
    fn of(window: &mut Window, cx: &App) -> Self {
        let metrics = terminal_ui::cell_metrics(window, cx);
        Self {
            family: metrics.font.family.to_string(),
            size: TerminalFontSettings::clamp_size(f32::from(metrics.font_size)),
        }
    }
}

/// What the page's fields hold.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    family: String,
    fallback: Option<String>,
    size: f32,
    line_spacing: f32,
    column_spacing: f32,
}

impl Shown {
    /// What the settings give: a field not chosen is the inherited font or 1.
    fn of(settings: &TerminalFontSettings, inherited: &Inherited) -> Self {
        Self {
            family: settings
                .family
                .clone()
                .unwrap_or_else(|| inherited.family.clone()),
            fallback: settings.fallback_family().map(str::to_owned),
            size: settings
                .size
                .map(TerminalFontSettings::clamp_size)
                .unwrap_or(inherited.size),
            line_spacing: settings
                .line_spacing
                .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                    TerminalFontSettings::clamp_spacing(s)
                }),
            column_spacing: settings
                .column_spacing
                .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                    TerminalFontSettings::clamp_spacing(s)
                }),
        }
    }

    /// The settings to store: a field the person did not change keeps what
    /// is stored (still inherited when it was), a changed one is chosen.
    fn settings(
        &self,
        stored: &TerminalFontSettings,
        inherited: &Inherited,
    ) -> TerminalFontSettings {
        let before = Self::of(stored, inherited);
        fn pick<T: PartialEq>(now: T, before: T, stored: Option<T>) -> Option<T> {
            if now == before { stored } else { Some(now) }
        }
        TerminalFontSettings {
            family: pick(self.family.clone(), before.family, stored.family.clone()),
            size: pick(self.size, before.size, stored.size),
            line_spacing: pick(self.line_spacing, before.line_spacing, stored.line_spacing),
            column_spacing: pick(
                self.column_spacing,
                before.column_spacing,
                stored.column_spacing,
            ),
            fallback: if self.fallback == before.fallback {
                stored.fallback.clone()
            } else {
                self.fallback.clone()
            },
        }
    }
}

/// The page: the combo boxes and the number fields as typed.
pub struct TerminalPage {
    inherited: Inherited,
    family: Entity<FamilyCombo>,
    fallback: Entity<FamilyCombo>,
    size: Entity<Editor>,
    line_spacing: Entity<Editor>,
    column_spacing: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inherited = Inherited::of(window, cx);
        let shown = Shown::of(&settings_ui::store::get(cx), &inherited);
        let focus = cx.focus_handle();
        let combo = |spec: ComboSpec, value: Option<String>, cx: &mut Context<Self>| {
            let focus = focus.clone();
            cx.new(|cx| FamilyCombo::new(spec, value, focus, cx))
        };
        let family = combo(FAMILY_COMBO, Some(shown.family.clone()), cx);
        let fallback = combo(FALLBACK_COMBO, shown.fallback.clone(), cx);
        let number = |text: String, window: &mut Window, cx: &mut Context<Self>| {
            editor::field::new(&text, None, true, window, cx)
        };
        let size = number(TerminalFontSettings::format_size(shown.size), window, cx);
        let line_spacing = number(
            TerminalFontSettings::format_spacing(shown.line_spacing),
            window,
            cx,
        );
        let column_spacing = number(
            TerminalFontSettings::format_spacing(shown.column_spacing),
            window,
            cx,
        );
        let edited = |_: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
            if matches!(e, EditorEvent::Edited(_)) {
                cx.notify();
            }
        };
        let chosen = |_: &mut Self,
                      _: Entity<FamilyCombo>,
                      _: &ComboEvent,
                      cx: &mut Context<Self>| { cx.notify() };
        let subscriptions = vec![
            cx.subscribe(&size, edited),
            cx.subscribe(&line_spacing, edited),
            cx.subscribe(&column_spacing, edited),
            cx.subscribe(&family, chosen),
            cx.subscribe(&fallback, chosen),
        ];
        Self {
            inherited,
            family,
            fallback,
            size,
            line_spacing,
            column_spacing,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn text(field: &Entity<Editor>, cx: &App) -> String {
        field.read(cx).text().trim().to_owned()
    }

    /// The fields as they are now; a number that is no number is the
    /// default, and out of range it is kept in range.
    fn draft(&self, cx: &App) -> Shown {
        Shown {
            family: self
                .family
                .read(cx)
                .value()
                .unwrap_or(&self.inherited.family)
                .to_owned(),
            fallback: self.fallback.read(cx).value().map(str::to_owned),
            size: TerminalFontSettings::parse_size(&Self::text(&self.size, cx)),
            line_spacing: TerminalFontSettings::parse_spacing(&Self::text(&self.line_spacing, cx)),
            column_spacing: TerminalFontSettings::parse_spacing(&Self::text(
                &self.column_spacing,
                cx,
            )),
        }
    }

    fn stored(&self, cx: &App) -> TerminalFontSettings {
        settings_ui::store::get(cx)
    }

    /// Puts `shown` in the fields.
    fn show(&mut self, shown: &Shown, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.family
            .update(cx, |c, cx| c.set_value(Some(shown.family.clone()), cx));
        self.fallback
            .update(cx, |c, cx| c.set_value(shown.fallback.clone(), cx));
        let size = TerminalFontSettings::format_size(shown.size);
        let line = TerminalFontSettings::format_spacing(shown.line_spacing);
        let column = TerminalFontSettings::format_spacing(shown.column_spacing);
        self.size.update(cx, |e, cx| e.set_text_selected(&size, cx));
        self.line_spacing
            .update(cx, |e, cx| e.set_text_selected(&line, cx));
        self.column_spacing
            .update(cx, |e, cx| e.set_text_selected(&column, cx));
        let mut window = window;
        for combo in [&self.family, &self.fallback] {
            combo.update(cx, |c, cx| c.close(window.as_deref_mut(), cx));
        }
        cx.notify();
    }

    fn number_field(&self, name: &'static str, window: &Window, cx: &App) -> AnyElement {
        let field = match name {
            field::SIZE => &self.size,
            field::LINE_HEIGHT => &self.line_spacing,
            _ => &self.column_spacing,
        };
        div()
            .id(name)
            .debug_selector(move || format!("terminal-font:{name}"))
            .flex_none()
            .w(px(NUMBER_WIDTH))
            .child(
                editor::field::frame(field, true, false, window, cx)
                    .h(px(ROW_HEIGHT))
                    .pl(px(6.0))
                    .pr(px(4.0)),
            )
            .into_any_element()
    }
}

impl SettingsPage for TerminalPage {
    fn is_modified(&self, cx: &App) -> bool {
        let stored = self.stored(cx);
        self.draft(cx).settings(&stored, &self.inherited) != stored
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let stored = self.stored(cx);
        let settings = self.draft(cx).settings(&stored, &self.inherited);
        settings_ui::store::set(cx, &settings);
        // The fields show the numbers kept in range.
        let shown = Shown::of(&settings, &self.inherited);
        self.show(&shown, Some(window), cx);
        Ok(())
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inherited = Inherited::of(window, cx);
        let shown = Shown::of(&self.stored(cx), &self.inherited);
        self.show(&shown, Some(window), cx);
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let shown = self.draft(cx);
        let open = [
            (field::FAMILY, &self.family),
            (field::FALLBACK, &self.fallback),
        ]
        .into_iter()
        .find(|(_, c)| c.read(cx).is_open())
        .map(|(name, c)| (name, c.read(cx).state()));
        json!({
            "family": shown.family,
            "fallback": shown.fallback.clone().unwrap_or_else(|| MONOSPACED.to_owned()),
            "size": Self::text(&self.size, cx),
            "line-height": Self::text(&self.line_spacing, cx),
            "column-width": Self::text(&self.column_spacing, cx),
            "inherited": {"family": self.inherited.family, "size": self.inherited.size},
            "list": open.as_ref().map(|(name, _)| name),
            "rows": open.as_ref().map(|(_, state)| state["rows"].clone()),
            "highlighted": open.as_ref().map(|(_, state)| state["highlighted"].clone()),
        })
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || match value {
            Json::String(s) => Ok(s.clone()),
            Json::Number(n) => Ok(n.to_string()),
            _ => Err(format!("`{name}` takes a string or a number")),
        };
        match name {
            field::FAMILY => {
                let family = value
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("`family` takes a family name")?
                    .to_owned();
                self.family
                    .update(cx, |c, cx| c.set_value(Some(family), cx));
            }
            field::FALLBACK => {
                let fallback = match value {
                    Json::Null => None,
                    Json::String(s) if s.is_empty() || s == MONOSPACED => None,
                    Json::String(s) => Some(s.clone()),
                    _ => return Err("`fallback` takes a family name or null".into()),
                };
                self.fallback.update(cx, |c, cx| c.set_value(fallback, cx));
            }
            field::SIZE | field::LINE_HEIGHT | field::COLUMN_WIDTH => {
                let t = text()?;
                let target = match name {
                    field::SIZE => &self.size,
                    field::LINE_HEIGHT => &self.line_spacing,
                    _ => &self.column_spacing,
                };
                target.update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            // Opens (`"family"`, `"fallback"`) or closes (`null`) a list.
            field::LIST => {
                for combo in [&self.family, &self.fallback] {
                    combo.update(cx, |c, cx| c.close(Some(&mut *window), cx));
                }
                match value.as_str() {
                    Some(field::FAMILY) => self.family.update(cx, |c, cx| c.open(window, cx)),
                    Some(field::FALLBACK) => self.fallback.update(cx, |c, cx| c.open(window, cx)),
                    None => {}
                    Some(other) => return Err(format!("no list `{other}`")),
                }
            }
            // Chooses a row of the open list by its label.
            field::CHOOSE => {
                let label = value.as_str().ok_or("`choose` takes a row's label")?;
                let combo = [&self.family, &self.fallback]
                    .into_iter()
                    .find(|c| c.read(cx).is_open())
                    .ok_or_else(|| format!("no row `{label}` in an open list"))?
                    .clone();
                combo.update(cx, |c, cx| c.choose_label(label, window, cx))?;
            }
            _ => return Err(format!("no field `{name}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for TerminalPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str, cx: &App| settings_ui::search::label(text, cx);
        let font_row = div()
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .child(label(FONT, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.family.clone())
            .child(div().w(px(CELL_GAP)))
            .child(label(FALLBACK, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.fallback.clone());
        let numbers_row = div()
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .child(label(SIZE, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::SIZE, window, cx))
            .child(div().w(px(CELL_GAP)))
            .child(label(LINE_HEIGHT, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::LINE_HEIGHT, window, cx))
            .child(div().w(px(CELL_GAP)))
            .child(label(COLUMN_WIDTH, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(field::COLUMN_WIDTH, window, cx));
        div()
            .id("terminal-page")
            .debug_selector(|| "terminal-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(ui::form::group_header(FONT_SETTINGS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(font_row)
                    .child(numbers_row),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inherited() -> Inherited {
        Inherited {
            family: "JetBrains Mono".into(),
            size: 13.0,
        }
    }

    #[test]
    fn a_field_left_alone_stays_inherited() {
        let none = TerminalFontSettings::default();
        let shown = Shown::of(&none, &inherited());
        assert_eq!(shown.family, "JetBrains Mono");
        assert_eq!(shown.size, 13.0);
        assert_eq!(shown.line_spacing, 1.0);
        assert_eq!(shown.fallback, None);
        // Nothing changed: nothing is stored.
        assert_eq!(shown.settings(&none, &inherited()), none);

        // Only the size changed: only the size is chosen.
        let chosen = Shown {
            size: 15.0,
            ..shown
        };
        assert_eq!(
            chosen.settings(&none, &inherited()),
            TerminalFontSettings {
                size: Some(15.0),
                ..none.clone()
            }
        );
    }

    #[test]
    fn a_chosen_field_stays_chosen_and_the_logical_fallback_is_none() {
        let stored = TerminalFontSettings {
            family: Some("Menlo".into()),
            fallback: Some("Monospaced".into()),
            column_spacing: Some(1.1),
            ..TerminalFontSettings::default()
        };
        let shown = Shown::of(&stored, &inherited());
        assert_eq!(shown.family, "Menlo");
        assert_eq!(shown.fallback, None);
        assert_eq!(shown.column_spacing, 1.1);
        // Applied as it is, the stored values are kept as they are.
        assert_eq!(shown.settings(&stored, &inherited()), stored);

        // Another fallback is chosen; the family goes back to the editor's.
        let changed = Shown {
            family: "JetBrains Mono".into(),
            fallback: Some("Symbols Nerd Font Mono".into()),
            ..shown
        };
        let settings = changed.settings(&stored, &inherited());
        assert_eq!(settings.family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(settings.fallback.as_deref(), Some("Symbols Nerd Font Mono"));
        assert_eq!(settings.column_spacing, Some(1.1));
    }
}
