//! The side panel of a grid (the reference's value editor: Shift+Enter,
//! Show Record View, Show Aggregate View; shots DB42, DB43, DB45b, DB47):
//! tabs Record, Value and Aggregates at the right of the cells.
//!
//! Record lists the current row as column names over their values. Value
//! shows the whole value of the current cell in a read-only editor,
//! soft-wrapped, JSON indented (toggles in the header); a value the page
//! cut at the fetch limit offers Load full value and Save to file over it.
//! Aggregates lists count, sum, average, minimum, maximum, median and the
//! coefficient of variation of the selected cells.

use editor::Editor;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FontWeight, IntoElement, MouseButton, Window,
    div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, header_tab, small_icon_button, toggle_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::data::{KeyMark, Pos};
use super::format::NULL_TEXT;
use super::value;
use super::values::Full;
use super::{TableGrid, aggregate};
use crate::icons;

/// Width of the panel (the reference's at its main window of 1440 px).
pub(super) const PANEL_WIDTH: f32 = 410.0;
const HEADER_HEIGHT: f32 = 36.0;
const ROW_HEIGHT: f32 = 26.0;
const NAME_WIDTH: f32 = 101.0;
const FIELD_HEIGHT: f32 = 27.0;

/// The panel's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTab {
    Record,
    Value,
    Aggregates,
}

impl PanelTab {
    pub const ALL: [PanelTab; 3] = [PanelTab::Record, PanelTab::Value, PanelTab::Aggregates];

    pub fn title(self) -> &'static str {
        match self {
            PanelTab::Record => "Record",
            PanelTab::Value => "Value",
            PanelTab::Aggregates => "Aggregates",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            PanelTab::Record => "record",
            PanelTab::Value => "value",
            PanelTab::Aggregates => "aggregates",
        }
    }
}

/// What the Value tab's editor holds: the page (its address), the cell,
/// the text's length and the JSON flags.
pub(super) type Shown = (usize, Option<Pos>, usize, bool, bool);

/// The open panel.
pub(super) struct Panel {
    pub tab: PanelTab,
    /// The Value tab's text.
    pub editor: Entity<Editor>,
    /// Indent JSON.
    pub json: bool,
    pub wrap: bool,
    /// The current value is JSON (the toggle works).
    pub is_json: bool,
    pub shown: Option<Shown>,
}

impl Panel {
    pub fn new(tab: PanelTab, window: &mut Window, cx: &mut Context<TableGrid>) -> Self {
        let editor = cx.new(|cx| {
            let mut buffer = text::Buffer::new("");
            buffer.set_read_only(true);
            let mut e = Editor::new(buffer, window, cx);
            e.set_viewer(true);
            e.set_bare(true);
            e.set_soft_wraps(true, cx);
            e
        });
        Self {
            tab,
            editor,
            json: true,
            wrap: true,
            is_json: false,
            shown: None,
        }
    }

    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "tab": self.tab.id(),
            "json": self.json,
            "wrap": self.wrap,
            "is_json": self.is_json,
            "text": (self.tab == PanelTab::Value).then(|| self.editor.read(cx).text()),
        })
    }
}

/// `1048576` -> `1.0 MiB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KiB", "MiB", "GiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

impl TableGrid {
    pub(super) fn render_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (tab, json_on, wrap_on, is_json) = {
            let p = self.values.panel.as_ref()?;
            (p.tab, p.json, p.wrap, p.is_json)
        };
        let body = match tab {
            PanelTab::Record => self.render_record(cx),
            PanelTab::Value => self.render_value(cx),
            PanelTab::Aggregates => self.render_aggregates(cx),
        };
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let text_size = theme.ui_size();
        let tabs = PanelTab::ALL.map(|t| {
            header_tab(("grid-panel-tab", t as usize), t == tab, true, cx)
                .px(px(12.0))
                .text_size(text_size)
                .debug_selector(move || format!("grid-panel-tab-{}", t.id()))
                .child(t.title())
                .on_click(
                    cx.listener(move |g, _: &ClickEvent, window, cx| g.open_panel(t, window, cx)),
                )
        });
        let mut header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(HEADER_HEIGHT))
            .px(px(8.0))
            .gap(px(2.0))
            .children(tabs)
            .child(div().flex_1());
        if tab == PanelTab::Value {
            header = header
                .child(
                    toggle_button(
                        "grid-panel-json",
                        IconName::JSON_OBJECT,
                        json_on && is_json,
                        cx,
                    )
                    .when(!is_json, |d| d.opacity(DISABLED_ALPHA))
                    .when(is_json, |d| {
                        d.on_click(cx.listener(|g, _: &ClickEvent, _, cx| g.toggle_json(cx)))
                    })
                    .tooltip(ui::components::Tooltip::text("Format JSON", None)),
                )
                .child(
                    toggle_button("grid-panel-wrap", IconName::SOFT_WRAP, wrap_on, cx)
                        .on_click(cx.listener(|g, _: &ClickEvent, _, cx| g.toggle_wrap(cx)))
                        .tooltip(ui::components::Tooltip::text("Soft-Wrap", None)),
                );
        }
        header = header
            .child(small_icon_button(
                "grid-panel-settings",
                IconName::SETTINGS_SMALL,
                false,
                cx,
            ))
            .child(
                small_icon_button("grid-panel-close", IconName::CLOSE_SMALL, true, cx)
                    .tooltip(ui::components::Tooltip::text("Close", None))
                    .on_click(
                        cx.listener(|g, _: &ClickEvent, window, cx| g.close_panel(window, cx)),
                    ),
            );
        Some(
            div()
                .id("grid-panel")
                .debug_selector(|| "grid-panel".to_owned())
                .flex()
                .flex_col()
                .flex_none()
                .w(px(PANEL_WIDTH))
                .h_full()
                .min_h(px(0.0))
                .border_l_1()
                .border_color(border)
                .bg(theme.color(UiRole::EditorBackground))
                .text_size(text_size)
                .text_color(theme.color(UiRole::Text))
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }

    fn render_aggregates(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let Some(a) = self.aggregates() else {
            return div()
                .px(px(14.0))
                .pt(px(6.0))
                .text_color(muted)
                .child("Select cells to see their aggregates.")
                .into_any_element();
        };
        let rows = a.lines().into_iter().map(|(name, value)| {
            let dim = value == aggregate::NOT_ENOUGH;
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(ROW_HEIGHT))
                .child(
                    div()
                        .w(px(NAME_WIDTH))
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(name),
                )
                .child(
                    div()
                        .id(name)
                        .debug_selector(move || format!("grid-aggregate-{name}"))
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .when(dim, |d| d.text_color(muted))
                        .child(value),
                )
        });
        div()
            .px(px(14.0))
            .pt(px(6.0))
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }

    fn render_record(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let border = theme.color(UiRole::BorderInput);
        let (Some(page), Some((row, _))) = (self.page.clone(), self.data_pos()) else {
            return div()
                .px(px(14.0))
                .pt(px(6.0))
                .text_color(muted)
                .child("Select a row to see it as a record.")
                .into_any_element();
        };
        let current = self.data_pos().map(|p| p.1);
        let mut fields = Vec::new();
        for (i, column) in page.columns.iter().enumerate() {
            let icon = match column.key {
                KeyMark::Primary => icons::COLUMN_KEY,
                KeyMark::Foreign => icons::COLUMN_FOREIGN,
                KeyMark::None => icons::COLUMN,
            };
            let cell = page.cell(row, i).cloned().flatten();
            let is_null = cell.is_none();
            let shown = cell
                .as_deref()
                .map(|t| t.lines().next().unwrap_or("").to_owned())
                .unwrap_or_else(|| NULL_TEXT.to_owned());
            fields.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(Icon::new(icon).size(px(16.0)))
                            .child(
                                div()
                                    .when(current == Some(i), |d| d.font_weight(FontWeight::BOLD))
                                    .child(column.name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .id(("grid-record-field", i))
                            .debug_selector(move || format!("grid-record-field-{i}"))
                            .h(px(FIELD_HEIGHT))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(border)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .when(is_null, |d| d.text_color(muted))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |g, _, window, cx| {
                                    window.focus(&g.focus);
                                    g.select_data_cell((row, i), cx)
                                }),
                            )
                            .child(shown),
                    ),
            );
        }
        div()
            .id("grid-record")
            .flex_1()
            .min_h(px(0.0))
            .px(px(8.0))
            .pt(px(6.0))
            .pb(px(12.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .overflow_y_scroll()
            .children(fields)
            .into_any_element()
    }

    fn render_value(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = &self.values.panel else {
            return div().into_any_element();
        };
        let banner = self.value_banner(cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .children(banner)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(panel.editor.clone()),
            )
            .into_any_element()
    }

    /// Over the Value tab's text: a cut value's size with its two actions,
    /// the read in progress, or why it failed.
    fn value_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pos = self.data_pos()?;
        let page = self.page.as_ref()?;
        let abs = page.first_row + pos.0 as u64;
        let full = self.values.full.get(&(abs, pos.1));
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let danger = theme.color(UiRole::Danger);
        let row = || {
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .px(px(14.0))
                .py(px(6.0))
        };
        let load = ui::link::link("grid-load-full", "Load full value", cx)
            .debug_selector(|| "grid-load-full".to_owned())
            .on_click(cx.listener(|g, _: &ClickEvent, _, cx| {
                if let Some(pos) = g.data_pos() {
                    g.load_full(pos, None, cx)
                }
            }));
        let save = ui::link::link("grid-save-value", "Save to file\u{2026}", cx)
            .debug_selector(|| "grid-save-value".to_owned())
            .on_click(cx.listener(|g, _: &ClickEvent, _, cx| g.save_value(None, cx)));
        match full {
            Some(Full::Loading { .. }) => {
                Some(
                    row()
                        .text_color(muted)
                        .child("Loading the full value\u{2026}")
                        .child(ui::link::link("grid-cancel-load", "Cancel", cx).on_click(
                            cx.listener(|g, _: &ClickEvent, _, cx| {
                                g.cancel_full();
                                cx.notify();
                            }),
                        ))
                        .into_any_element(),
                )
            }
            Some(Full::Failed(e)) => Some(
                row()
                    .child(
                        div()
                            .text_color(danger)
                            .child(format!("Cannot read the value: {e}")),
                    )
                    .child(load)
                    .into_any_element(),
            ),
            _ => {
                let v = self.cell_value(pos)?;
                let value::Cut { shown, full_len } = value::cut_of(&v)?;
                let size = match full_len {
                    Some(n) => format!(
                        "The value is cut at {} ({} in all)",
                        human_size(shown as u64),
                        human_size(n)
                    ),
                    None => format!("The value is cut at {}", human_size(shown as u64)),
                };
                Some(
                    row()
                        .child(div().text_color(muted).child(size))
                        .child(load)
                        .child(save)
                        .into_any_element(),
                )
            }
        }
    }
}
