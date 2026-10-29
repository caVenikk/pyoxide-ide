//! Settings > Tools > Database > Query Execution: what the Execute actions
//! run (a list of Execute, Execute (2) and Execute (3), each with its
//! shortcut, what it runs with the caret inside a statement or outside, for
//! a selection, and whether the result opens in a new tab), how a script is
//! split and the checks around a run (review of parameters, the execution
//! time in the editor, the warning before an unsafe query).
//!
//! Wired: the caret and selection choices and "Open results in new tab" of
//! every Execute action, the three checkboxes. Stored only: "Split a
//! script".

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, ElementId, Entity, FocusHandle,
    IntoElement, Pixels, Render, SharedString, Subscription, Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::commands::EditField;
use settings_ui::{PageContext, PageSpec, Scope, SettingsPage};
use themes::UiRole;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

use crate::query_settings::{
    self, CaretInside, CaretOutside, Choice, ForSelection, QueryExecutionSettings, SplitScript,
    Variant,
};

/// The Tools group lives in the workbench's pages.
const TOOLS: &str = workbench::settings::TOOLS;
pub const DATABASE: &str = "tools.database";
pub const QUERY_EXECUTION: &str = "tools.database.query-execution";

const SHORTCUT: &str = "Shortcut:";
const CARET_INSIDE: &str = "When caret inside statement execute:";
const CARET_OUTSIDE: &str = "When caret outside statement execute:";
const FOR_SELECTION: &str = "For selection execute:";
const NEW_TAB: &str = "Open results in new tab";
const SPLIT_SCRIPT: &str = "Split a script for execution in Generic and ANSI SQL dialects:";
const REVIEW: &str = "Review parameters before execution";
const SHOW_TIME: &str = "Show query execution time in the editor after query completion";
const WARN_UNSAFE: &str = "Show warning before running potentially unsafe queries";
const SEARCH_TERMS: &[&str] = &[
    "Execute",
    "Execute (2)",
    "Execute (3)",
    SHORTCUT,
    CARET_INSIDE,
    CARET_OUTSIDE,
    FOR_SELECTION,
    NEW_TAB,
    SPLIT_SCRIPT,
    REVIEW,
    SHOW_TIME,
    WARN_UNSAFE,
];

const ROW_HEIGHT: f32 = 22.0;
const ROW_GAP: f32 = 6.0;
const COMBO_HEIGHT: f32 = 28.0;
const COMBO_WIDTH: f32 = 235.0;
const SPLIT_COMBO_WIDTH: f32 = 350.0;
const LABEL_WIDTH: f32 = 245.0;
const LIST_WIDTH: f32 = 125.0;
const LIST_GAP: f32 = 12.0;
const LIST_ROW_HEIGHT: f32 = 24.0;
const SECTION_GAP: f32 = 14.0;

/// The keys of `settings.edit`: `execute.caret-inside`,
/// `execute-2.new-tab`, ... for the options of one Execute action, and
/// these for the page.
mod field {
    pub const TAB: &str = "tab";
    pub const SPLIT_SCRIPT: &str = "split-script";
    pub const REVIEW: &str = "review-parameters";
    pub const SHOW_TIME: &str = "show-execution-time";
    pub const WARN_UNSAFE: &str = "warn-unsafe-queries";
    pub const CARET_INSIDE: &str = "caret-inside";
    pub const CARET_OUTSIDE: &str = "caret-outside";
    pub const FOR_SELECTION: &str = "for-selection";
    pub const NEW_TAB: &str = "new-tab";
}

fn variant_key(variant: Variant) -> &'static str {
    match variant {
        Variant::Execute => "execute",
        Variant::Execute2 => "execute-2",
        Variant::Execute3 => "execute-3",
    }
}

fn variant_from_key(key: &str) -> Option<Variant> {
    Variant::ALL.into_iter().find(|v| variant_key(*v) == key)
}

pub fn pages() -> Vec<PageSpec> {
    vec![
        PageSpec {
            weight: 100,
            ..PageSpec::group(DATABASE, Some(TOOLS), "Database")
        },
        PageSpec {
            id: QUERY_EXECUTION,
            parent: Some(DATABASE),
            title: "Query Execution",
            weight: 100,
            scope: Scope::Application,
            search_terms: SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(QueryExecutionPage::new)),
        },
    ]
}

/// The combo boxes of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combo {
    CaretInside,
    CaretOutside,
    ForSelection,
    SplitScript,
}

impl Combo {
    const ALL: [Combo; 4] = [
        Combo::CaretInside,
        Combo::CaretOutside,
        Combo::ForSelection,
        Combo::SplitScript,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    fn name(self) -> &'static str {
        match self {
            Combo::CaretInside => field::CARET_INSIDE,
            Combo::CaretOutside => field::CARET_OUTSIDE,
            Combo::ForSelection => field::FOR_SELECTION,
            Combo::SplitScript => field::SPLIT_SCRIPT,
        }
    }
}

/// Database > Query Execution.
pub struct QueryExecutionPage {
    draft: QueryExecutionSettings,
    /// The Execute action whose options the right side shows.
    variant: Variant,
    list: Option<(Combo, Entity<ContextMenu>, Subscription)>,
    anchors: Rc<[Cell<Bounds<Pixels>>; 4]>,
    focus: FocusHandle,
}

impl QueryExecutionPage {
    fn new(_: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            draft: query_settings::get(cx),
            variant: Variant::Execute,
            list: None,
            anchors: Rc::default(),
            focus: cx.focus_handle(),
        }
    }

    fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        let flag = match name {
            field::REVIEW => self.draft.review_parameters,
            field::SHOW_TIME => self.draft.show_execution_time,
            field::WARN_UNSAFE => self.draft.warn_unsafe_queries,
            field::NEW_TAB => self.draft.options(self.variant).new_tab,
            _ => return,
        };
        let _ = self.set_flag(self.variant, name, !flag);
        cx.notify();
    }

    fn set_flag(&mut self, variant: Variant, name: &str, flag: bool) -> Result<(), String> {
        match name {
            field::REVIEW => self.draft.review_parameters = flag,
            field::SHOW_TIME => self.draft.show_execution_time = flag,
            field::WARN_UNSAFE => self.draft.warn_unsafe_queries = flag,
            field::NEW_TAB => self.draft.options_mut(variant).new_tab = flag,
            _ => return Err(format!("no field `{name}` on this page")),
        }
        Ok(())
    }

    /// Sets a combo box's option by its stored id.
    fn set_choice(&mut self, variant: Variant, name: &str, id: &str) -> Result<(), String> {
        fn pick<C: Choice>(id: &str) -> Result<C, String> {
            C::from_id(id).ok_or_else(|| format!("unknown option `{id}`"))
        }
        match name {
            field::CARET_INSIDE => {
                self.draft.options_mut(variant).caret_inside = pick::<CaretInside>(id)?
            }
            field::CARET_OUTSIDE => {
                self.draft.options_mut(variant).caret_outside = pick::<CaretOutside>(id)?
            }
            field::FOR_SELECTION => {
                self.draft.options_mut(variant).for_selection = pick::<ForSelection>(id)?
            }
            field::SPLIT_SCRIPT => self.draft.split_script = pick::<SplitScript>(id)?,
            _ => return Err(format!("no field `{name}` on this page")),
        }
        Ok(())
    }

    // ---- the combo boxes' lists ------------------------------------------------

    /// The list of a combo box: its options and the index of the current one.
    fn options_of(&self, combo: Combo) -> (Vec<(&'static str, &'static str)>, Option<usize>) {
        fn rows<C: Choice>(current: C) -> (Vec<(&'static str, &'static str)>, Option<usize>) {
            (
                C::ALL.iter().map(|c| (c.id(), c.label())).collect(),
                C::ALL.iter().position(|c| *c == current),
            )
        }
        let options = self.draft.options(self.variant);
        match combo {
            Combo::CaretInside => rows(options.caret_inside),
            Combo::CaretOutside => rows(options.caret_outside),
            Combo::ForSelection => rows(options.for_selection),
            Combo::SplitScript => rows(self.draft.split_script),
        }
    }

    fn field_of(&self, combo: Combo) -> String {
        match combo {
            Combo::SplitScript => field::SPLIT_SCRIPT.to_owned(),
            other => format!("{}.{}", variant_key(self.variant), other.name()),
        }
    }

    fn open_list(&mut self, combo: Combo, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        let (rows, selected) = self.options_of(combo);
        let name = self.field_of(combo);
        let items = rows
            .into_iter()
            .map(|(id, label)| {
                MenuItem::action(
                    label,
                    EditField {
                        field: name.clone(),
                        value: json!(id),
                    },
                )
            })
            .collect();
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                selected,
                min_width: Some(f32::from(self.anchors[combo.index()].get().size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((combo, menu, subscription));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            cx.notify();
        }
    }

    // ---- drawing -----------------------------------------------------------------

    fn label(text: &'static str, cx: &App) -> AnyElement {
        settings_ui::search::spotlight(text, div().whitespace_nowrap().child(text), cx)
    }

    fn check(
        &self,
        name: &'static str,
        checked: bool,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = ui::form::check_row(
            ElementId::Name(format!("database-row:{name}").into()),
            ElementId::Name(format!("database-check:{name}").into()),
            checked,
            div().whitespace_nowrap().child(label),
            cx,
        )
        .debug_selector(move || format!("database:{name}"))
        .min_h(px(ROW_HEIGHT))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(name, cx)));
        settings_ui::search::spotlight(label, row, cx)
    }

    fn combo(&self, combo: Combo, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (rows, selected) = self.options_of(combo);
        let text = selected
            .and_then(|i| rows.get(i))
            .map_or("", |(_, label)| *label);
        let open = self.list.as_ref().is_some_and(|(c, ..)| *c == combo);
        let anchors = self.anchors.clone();
        let name = combo.name();
        ui::form::combo(
            SharedString::from(format!("database-combo:{name}")),
            vec![div().child(text).into_any_element()],
            cx,
        )
        .debug_selector(move || format!("database:{name}"))
        .w(px(width))
        .h(px(COMBO_HEIGHT))
        .pointing_hand()
        .when(open, |d| d.border_color(theme.color(UiRole::Focus)))
        .child(
            canvas(
                move |bounds, _, _| anchors[combo.index()].set(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            if this.list.as_ref().is_some_and(|(c, ..)| *c == combo) {
                this.close_list(cx);
                window.focus(&this.focus);
            } else {
                this.open_list(combo, window, cx);
            }
        }))
        .into_any_element()
    }

    /// The list of the Execute actions, each with its shortcut.
    fn variants(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows: Vec<AnyElement> = Variant::ALL
            .into_iter()
            .map(|variant| {
                let selected = variant == self.variant;
                let shortcut = shortcut_of(variant, cx).unwrap_or_default();
                let id = variant_key(variant);
                let mut row = div()
                    .id(SharedString::from(format!("database-variant:{id}")))
                    .debug_selector(move || format!("database:variant:{id}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(LIST_ROW_HEIGHT))
                    .px(px(8.0))
                    .pointing_hand()
                    .child(div().whitespace_nowrap().child(variant.title()))
                    .child(
                        div()
                            .text_color(theme.color(UiRole::TextMuted))
                            .child(shortcut),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.variant = variant;
                        this.close_list(cx);
                        cx.notify();
                    }));
                if selected {
                    row = row
                        .bg(theme.color(UiRole::SelectionInactive))
                        .text_color(theme.color(UiRole::Text));
                }
                settings_ui::search::spotlight(variant.title(), row, cx)
            })
            .collect();
        div()
            .id("database-variants")
            .flex_none()
            .w(px(LIST_WIDTH))
            .flex()
            .flex_col()
            .p(px(2.0))
            .rounded(ui::form::field_radius(cx))
            .border_1()
            .border_color(theme.color(UiRole::BorderInput))
            .children(rows)
            .into_any_element()
    }

    /// A row of the right side: the label column and the field.
    fn form_row(label: &'static str, field: AnyElement, cx: &App) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .w(px(LABEL_WIDTH))
                    .child(Self::label(label, cx)),
            )
            .child(field)
            .into_any_element()
    }
}

/// The shortcut of the Execute action, as the keymap has it.
fn shortcut_of(variant: Variant, cx: &App) -> Option<String> {
    match variant {
        Variant::Execute => workbench::main_menu::shortcut(&crate::console::Execute, cx),
        Variant::Execute2 => workbench::main_menu::shortcut(&crate::console::Execute2, cx),
        Variant::Execute3 => workbench::main_menu::shortcut(&crate::console::Execute3, cx),
    }
}

impl SettingsPage for QueryExecutionPage {
    fn is_modified(&self, cx: &App) -> bool {
        self.draft != query_settings::get(cx)
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        query_settings::set(cx, &self.draft);
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = query_settings::get(cx);
        self.list = None;
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let mut state = serde_json::Map::new();
        state.insert(field::TAB.to_owned(), json!(variant_key(self.variant)));
        for variant in Variant::ALL {
            let o = self.draft.options(variant);
            state.insert(
                variant_key(variant).to_owned(),
                json!({
                    "shortcut": shortcut_of(variant, cx),
                    field::CARET_INSIDE: o.caret_inside.id(),
                    field::CARET_OUTSIDE: o.caret_outside.id(),
                    field::FOR_SELECTION: o.for_selection.id(),
                    field::NEW_TAB: o.new_tab,
                }),
            );
        }
        state.insert(
            field::SPLIT_SCRIPT.to_owned(),
            json!(self.draft.split_script.id()),
        );
        state.insert(
            field::REVIEW.to_owned(),
            json!(self.draft.review_parameters),
        );
        state.insert(
            field::SHOW_TIME.to_owned(),
            json!(self.draft.show_execution_time),
        );
        state.insert(
            field::WARN_UNSAFE.to_owned(),
            json!(self.draft.warn_unsafe_queries),
        );
        Json::Object(state)
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || {
            value
                .as_str()
                .ok_or_else(|| format!("`{name}` takes a string"))
        };
        let flag = || {
            value
                .as_bool()
                .ok_or_else(|| format!("`{name}` takes true or false"))
        };
        if name == field::TAB {
            self.variant = variant_from_key(text()?)
                .ok_or_else(|| "tab is `execute`, `execute-2` or `execute-3`".to_owned())?;
        } else if let Some((variant, key)) = name.split_once('.') {
            let variant = variant_from_key(variant)
                .ok_or_else(|| format!("no Execute action `{variant}` on this page"))?;
            match key {
                field::NEW_TAB => self.set_flag(variant, key, flag()?)?,
                _ => self.set_choice(variant, key, text()?)?,
            }
        } else {
            match name {
                field::SPLIT_SCRIPT => self.set_choice(self.variant, name, text()?)?,
                _ => self.set_flag(self.variant, name, flag()?)?,
            }
        }
        self.close_list(cx);
        cx.notify();
        Ok(())
    }
}

impl Render for QueryExecutionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = self.draft;
        let (link, divider) = {
            let theme = cx.theme();
            (theme.color(UiRole::TextLink), theme.color(UiRole::Divider))
        };
        let options = *draft.options(self.variant);
        let shortcut = shortcut_of(self.variant, cx).unwrap_or_default();
        let overlay = self.list.as_ref().map(|(combo, menu, _)| {
            ui::menu::overlay(
                menu,
                self.anchors[combo.index()].get().bottom_left(),
                window,
            )
        });
        let form = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(Self::form_row(
                SHORTCUT,
                div().text_color(link).child(shortcut).into_any_element(),
                cx,
            ))
            .child(Self::form_row(
                CARET_INSIDE,
                self.combo(Combo::CaretInside, COMBO_WIDTH, cx),
                cx,
            ))
            .child(Self::form_row(
                CARET_OUTSIDE,
                self.combo(Combo::CaretOutside, COMBO_WIDTH, cx),
                cx,
            ))
            .child(Self::form_row(
                FOR_SELECTION,
                self.combo(Combo::ForSelection, COMBO_WIDTH, cx),
                cx,
            ))
            .child(self.check(field::NEW_TAB, options.new_tab, NEW_TAB, cx));
        div()
            .id("database-query-execution-page")
            .debug_selector(|| "database-query-execution-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(SECTION_GAP))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(LIST_GAP))
                    .child(self.variants(cx))
                    .child(form),
            )
            .child(div().h(px(1.0)).bg(divider))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(Self::label(SPLIT_SCRIPT, cx))
                    .child(self.combo(Combo::SplitScript, SPLIT_COMBO_WIDTH, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(self.check(field::REVIEW, draft.review_parameters, REVIEW, cx))
                    .child(self.check(field::SHOW_TIME, draft.show_execution_time, SHOW_TIME, cx))
                    .child(self.check(
                        field::WARN_UNSAFE,
                        draft.warn_unsafe_queries,
                        WARN_UNSAFE,
                        cx,
                    )),
            )
            .children(overlay)
    }
}
