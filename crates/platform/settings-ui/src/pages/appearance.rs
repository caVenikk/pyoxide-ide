//! Appearance & Behavior > Appearance (`AppearanceConfigurable`): the
//! Theme combo box, which switches the theme at once (Cancel keeps it,
//! as in the reference), and the editor colour scheme below it, stored
//! with Apply (`EditorSchemesPanel` in the page). The theme list groups the
//! themes as the reference's (`ThemeListProviderImpl`: Islands, the other
//! bundled themes, "Custom" for imported ones) and ends with "Import
//! Theme…" in place of "Get More Themes…".
//!
//! Not drawn yet: Sync with OS and its gear, the scheme's gear, and the
//! Accessibility and UI Options groups.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, Entity, FocusHandle, IntoElement,
    Pixels, Render, SharedString, Subscription, Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{CatalogEntry, ThemeGroup, UiRole};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use crate::commands::{EditField, ImportTheme, SwitchTheme};
use crate::page::{PageContext, SettingsPage};
use crate::theme_switch::{self, IMPORT_THEME};

pub const ID: &str = "appearance";
pub const TITLE: &str = "Appearance";
/// The reference's labels (IdeBundle).
const THEME: &str = "Theme:";
const SCHEME: &str = "Editor color scheme:";
pub const SEARCH_TERMS: &[&str] = &[THEME, SCHEME];

/// The page's metrics, from the reference (Kotlin UI DSL): a combo box
/// 24 high, 7 after the theme's label and 14 after the scheme's (its
/// panel's own gap), the scheme row indented by 20, the rows 34 apart, the
/// scheme's combo box 193 wide (the theme's fits its longest name).
const COMBO_HEIGHT: f32 = 24.0;
const THEME_GAP: f32 = 7.0;
const SCHEME_GAP: f32 = 14.0;
const INDENT: f32 = 20.0;
const ROW_GAP: f32 = 10.0;
const SCHEME_WIDTH: f32 = 193.0;
/// A combo box around its text: the side paddings, the arrow with its gap
/// and the border (`ui::form::combo`).
const COMBO_CHROME: f32 = 8.0 + 6.0 + 16.0 + 8.0 + 2.0;

/// Which combo box's list is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    Theme,
    Scheme,
}

impl List {
    fn name(self) -> &'static str {
        match self {
            List::Theme => "theme",
            List::Scheme => "scheme",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The page: the scheme chosen here (not applied yet) and an open list.
pub struct AppearancePage {
    /// The editor colour scheme chosen on the page, with the theme it was
    /// chosen for; another theme drops it (the scheme follows the theme).
    draft: Option<(String, String)>,
    list: Option<(List, Entity<ContextMenu>, Subscription)>,
    /// Where the combo boxes are drawn (the lists open below them).
    anchors: Rc<RefCell<[Bounds<Pixels>; 2]>>,
    focus: FocusHandle,
}

impl AppearancePage {
    pub fn new(_: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            draft: None,
            list: None,
            anchors: Rc::default(),
            focus: cx.focus_handle(),
        }
    }

    /// The scheme chosen here for the theme in use, if any.
    fn draft_scheme(&self, cx: &App) -> Option<String> {
        let theme = theme_switch::current_theme(cx);
        self.draft
            .as_ref()
            .filter(|(t, _)| *t == theme)
            .map(|(_, s)| s.clone())
    }

    /// The scheme the combo box shows.
    fn scheme(&self, cx: &App) -> String {
        self.draft_scheme(cx)
            .unwrap_or_else(|| theme_switch::current_scheme(cx))
    }

    fn name_of(id: &str, cx: &App) -> SharedString {
        theme_switch::catalog(cx)
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.to_owned())
            .into()
    }

    /// Rows grouped as the reference's combo box: a line between groups,
    /// the title "Custom" above imported ones.
    fn grouped<'a>(
        entries: impl Iterator<Item = &'a CatalogEntry>,
        row: impl Fn(&CatalogEntry) -> MenuItem,
    ) -> Vec<MenuItem> {
        let mut items = Vec::new();
        let mut group: Option<ThemeGroup> = None;
        for entry in entries {
            if group != Some(entry.group) {
                match entry.group.title() {
                    Some(title) => items.push(MenuItem::header(title)),
                    None if group.is_some() => items.push(MenuItem::separator()),
                    None => {}
                }
            }
            group = Some(entry.group);
            items.push(row(entry));
        }
        items
    }

    fn items(list: List, cx: &App) -> (Vec<MenuItem>, Option<usize>) {
        let catalog = theme_switch::catalog(cx);
        match list {
            List::Theme => {
                let current = theme_switch::current_theme(cx);
                let mut items = Self::grouped(catalog.themes(), |e| {
                    MenuItem::action(
                        e.name.clone(),
                        SwitchTheme {
                            id: Some(e.id.clone()),
                        },
                    )
                });
                let selected = catalog.themes().position(|e| e.id == current);
                // Rows before it that are not themes (lines, titles).
                let selected = selected.and_then(|n| nth_action(&items, n));
                items.push(MenuItem::separator());
                items.push(MenuItem::action(IMPORT_THEME, ImportTheme { path: None }));
                (items, selected)
            }
            List::Scheme => {
                let items = Self::grouped(catalog.schemes(), |e| {
                    MenuItem::action(
                        e.name.clone(),
                        EditField {
                            field: "editor-scheme".into(),
                            value: json!(e.id),
                        },
                    )
                });
                (items, None)
            }
        }
    }

    fn open_list(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        let (items, mut selected) = Self::items(list, cx);
        if list == List::Scheme {
            let scheme = self.scheme(cx);
            let position = theme_switch::catalog(cx)
                .schemes()
                .position(|e| e.id == scheme);
            selected = position.and_then(|n| nth_action(&items, n));
        }
        let bounds = self.anchors.borrow()[list.index()];
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                selected,
                min_width: Some(f32::from(bounds.size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((list, menu, subscription));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_list(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.as_ref().is_some_and(|(l, _, _)| *l == list) {
            self.close_list(cx);
        } else {
            self.open_list(list, window, cx);
        }
    }

    fn set_draft(&mut self, scheme: &str, cx: &mut Context<Self>) -> Result<(), String> {
        if theme_switch::catalog(cx).get(scheme).is_none() {
            return Err(format!("unknown editor color scheme `{scheme}`"));
        }
        self.draft = Some((theme_switch::current_theme(cx), scheme.to_owned()));
        cx.notify();
        Ok(())
    }

    /// The width of the theme's combo box: its widest item, a theme name
    /// or "Import Theme…" (Swing's combo box fits its widest item, and its
    /// list is as wide as the box).
    fn theme_width(window: &Window, cx: &App) -> f32 {
        let theme = cx.theme();
        let font = gpui::font(theme.ui_family());
        let catalog = theme_switch::catalog(cx);
        let widest = catalog
            .themes()
            .map(|e| e.name.as_str())
            .chain([IMPORT_THEME])
            .map(|name| {
                let run = gpui::TextRun {
                    len: name.len(),
                    font: font.clone(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(name.to_owned()),
                    theme.ui_size(),
                    &[run],
                    None,
                );
                f32::from(line.width)
            })
            .fold(0.0, f32::max);
        (widest + COMBO_CHROME).ceil()
    }

    fn combo(
        &self,
        list: List,
        text: SharedString,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.list.as_ref().is_some_and(|(l, _, _)| *l == list);
        let anchors = self.anchors.clone();
        let focus = cx.theme().color(UiRole::Focus);
        ui::form::combo(
            SharedString::from(format!("appearance-{}", list.name())),
            vec![div().child(text).into_any_element()],
            cx,
        )
        .debug_selector(move || format!("appearance:{}", list.name()))
        .h(px(COMBO_HEIGHT))
        .w(px(width))
        .when(open, |d| d.border_color(focus))
        .child(
            canvas(
                move |bounds, _, _| anchors.borrow_mut()[list.index()] = bounds,
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_click(
            cx.listener(move |this, _: &ClickEvent, window, cx| this.toggle_list(list, window, cx)),
        )
        .into_any_element()
    }
}

/// The index among `items` of the `n`-th row that runs an action.
fn nth_action(items: &[MenuItem], n: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, MenuItem::Action { .. }))
        .nth(n)
        .map(|(ix, _)| ix)
}

impl SettingsPage for AppearancePage {
    fn is_modified(&self, cx: &App) -> bool {
        self.draft_scheme(cx)
            .is_some_and(|s| s != theme_switch::current_scheme(cx))
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(scheme) = self.draft_scheme(cx) {
            theme_switch::set_scheme(&scheme, cx)?;
        }
        self.draft = None;
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = None;
        self.close_list(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        json!({
            "theme": theme_switch::current_theme(cx),
            "editor-scheme": self.scheme(cx),
            "list": self.list.as_ref().map(|(l, _, _)| l.name()),
            "menu": self.list.as_ref().map(|(_, m, _)| m.read(cx).state_json()),
        })
    }

    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{field}` takes a string"))
        };
        match field {
            // The combo box switches the theme at once.
            "theme" => theme_switch::switch(&text()?, cx)?,
            "editor-scheme" => self.set_draft(&text()?, cx)?,
            // Opens (`"theme"`, `"scheme"`) or closes (`null`) a list.
            "list" => match value.as_str() {
                Some("theme") => self.open_list(List::Theme, window, cx),
                Some("scheme") => self.open_list(List::Scheme, window, cx),
                None => self.close_list(cx),
                Some(other) => return Err(format!("no list `{other}`")),
            },
            _ => return Err(format!("no field `{field}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for AppearancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_name = Self::name_of(&theme_switch::current_theme(cx), cx);
        let scheme_name = Self::name_of(&self.scheme(cx), cx);
        let theme_width = Self::theme_width(window, cx);
        let theme_row = div()
            .flex()
            .flex_row()
            .items_center()
            .child(crate::search::label(THEME, cx))
            .child(div().w(px(THEME_GAP)))
            .child(self.combo(List::Theme, theme_name, theme_width, cx));
        let scheme_row = div()
            .pl(px(INDENT))
            .flex()
            .flex_row()
            .items_center()
            .child(crate::search::label(SCHEME, cx))
            .child(div().w(px(SCHEME_GAP)))
            .child(self.combo(List::Scheme, scheme_name, SCHEME_WIDTH, cx));
        let overlay = self.list.as_ref().map(|(list, menu, _)| {
            let at = self.anchors.borrow()[list.index()].bottom_left();
            ui::menu::overlay(menu, at, window)
        });
        div()
            .id("appearance-page")
            .debug_selector(|| "appearance-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(theme_row)
            .child(scheme_row)
            .children(overlay)
    }
}
