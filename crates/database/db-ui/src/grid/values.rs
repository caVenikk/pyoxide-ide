//! Values in the grid (issue "grid value display, value panel and views",
//! scenarios grid-values and grid-copy-and-views): the commands and state
//! of the value panel, display types of binary columns, the views (Table,
//! Transpose, Text), loading the whole value of a cut cell and saving a
//! value to a file. What is drawn is in [`super::panel`] and
//! [`super::cell_menu`]; what a value looks like is in [`super::value`].

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use db_core::Value as DbValue;
use db_session::{CellQuery, JobId, PageError};
use editor::Editor;
use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Focusable as _, Task, Window};
use serde_json::{Value as Json, json};
use text::{Selection as TextSelection, SelectionSet};

use super::aggregate;
use super::data::{GridPage, KeyMark, Pos, Selection};
use super::formats::{self, DataFormats};
use super::panel::{Panel, PanelTab};
use super::value::{self, CellFormat, DisplayType};
use super::views::{self, ViewMode};
use super::{Grids, TableGrid, status, with_target};

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Open the value panel on the value of the current cell.
    ShowValue => "db.grid.value-editor", "Open in Value Editor",
        "Opens the side panel of the active table grid on its Value tab: the whole value of the current cell, soft-wrapped; a JSON value is indented.";
    /// Open the value panel on the current row.
    ShowRecord => "db.grid.record-view", "Show Record View",
        "Opens the side panel of the active table grid on its Record tab: the current row as column names and values.";
    /// Open the value panel on the aggregates.
    ShowAggregates => "db.grid.aggregate-view", "Show Aggregate View",
        "Opens the side panel of the active table grid on its Aggregates tab: count, sum, average, minimum, maximum and median of the selected cells.";
    /// Close the value panel.
    HidePanel => "db.grid.hide-panel", "Close Value Panel",
        "Closes the side panel of the active table grid.";
    /// Change how the page is shown.
    SetView { view: String } => "db.grid.set-view", "View As",
        "Shows the page of the active table grid as `table`, `transpose` (a row for each column) or `text` (comma separated lines).",
        r#"{"view": "text"}"#;
    /// Change how a binary column shows its values.
    SetDisplayType { column: usize, display: String } => "db.grid.set-display-type", "Change Display Type",
        "Sets how column `column` (zero-based) of the active table grid shows binary values: `detect`, `uuid`, `uuid-swapped`, `text`, `hex` or `hex-ascii`.",
        r#"{"column": 5, "display": "hex-ascii"}"#;
    /// Read the whole value of the current cell.
    LoadFullValue => "db.grid.load-full-value", "Load Full Value",
        "Reads the whole value of the current cell, which the page cut at the fetch limit, by the row's key, without blocking the window.";
    /// Save the value of the current cell to a file.
    SaveValue { path: Option<String> } => "db.grid.save-value", "Save to File",
        "Writes the value of the current cell (the whole value, read first when the page cut it) to `path`, or to a file chosen in a dialog.",
        r#"{"path": "/tmp/value.txt"}"#;
    /// Indent or not the JSON of the value panel.
    ToggleJson => "db.grid.value-format-json", "Format JSON",
        "Switches the Value tab between indented and as-sent JSON.";
    /// Soft-wrap or not the value panel.
    ToggleWrap => "db.grid.value-soft-wrap", "Soft-Wrap Value",
        "Switches soft wraps of the Value tab.";
    /// Copy one aggregate of the selection.
    CopyAggregation { name: Option<String> } => "db.grid.copy-aggregation", "Copy Aggregation Result",
        "Copies an aggregate (`SUM` when no `name`) of the selected cells of the active table grid to the clipboard.",
        r#"{"name": "AVG"}"#;
}

/// The whole value of a cell: asked for, read, or not readable.
pub(super) enum Full {
    Loading {
        job: JobId,
        /// Write it to this file when it arrives.
        save: Option<PathBuf>,
    },
    Loaded(Arc<DbValue>),
    Failed(String),
}

/// What the text view shows.
pub(super) struct TextView {
    pub editor: Entity<Editor>,
    /// The page (its address) and the hidden columns the text was made for.
    pub made_for: Option<(usize, u64)>,
}

/// The value features' state of a grid.
#[derive(Default)]
pub(super) struct ValueState {
    /// Display types of binary columns, by column name.
    pub display: HashMap<String, DisplayType>,
    pub panel: Option<Panel>,
    pub view: ViewMode,
    /// The page on its side while the view is Transpose.
    pub transposed: Option<Arc<GridPage>>,
    pub text: Option<TextView>,
    /// Whole values of cut cells by the cell in the result (row, column).
    pub full: HashMap<(u64, usize), Full>,
    pub reformat: Option<Task<()>>,
    /// The last save to a file: the path, or why it failed.
    pub saved: Option<Result<String, String>>,
}

impl TableGrid {
    /// What the cell texts of a page are made with now.
    pub(super) fn cell_format(&self, cx: &App) -> CellFormat {
        CellFormat {
            formats: Arc::new(settings_ui::store::get::<DataFormats>(cx)),
            display: self.values.display.clone(),
        }
    }

    /// The page the cells show: the data page, or on its side.
    pub(super) fn shown_page(&self) -> Option<&Arc<GridPage>> {
        if self.values.view == ViewMode::Transpose
            && let Some(t) = &self.values.transposed
        {
            return Some(t);
        }
        self.page.as_ref()
    }

    pub(super) fn is_transposed(&self) -> bool {
        self.values.view == ViewMode::Transpose && self.values.transposed.is_some()
    }

    /// The current cell in the data page (row, column).
    pub(super) fn data_pos(&self) -> Option<Pos> {
        let (r, c) = self.selection?.lead;
        if self.is_transposed() {
            (c >= 1).then_some((c - 1, r))
        } else {
            Some((r, c))
        }
    }

    /// Selects a cell of the data page, in whichever view.
    pub(super) fn select_data_cell(&mut self, (row, column): Pos, cx: &mut Context<Self>) {
        let pos = if self.is_transposed() {
            (column, row + 1)
        } else {
            (row, column)
        };
        self.select(Selection::cell(pos), cx);
    }

    /// The value of a data cell: the whole one when it was read.
    pub(super) fn cell_value(&self, (row, column): Pos) -> Option<Cow<'_, DbValue>> {
        let page = self.page.as_ref()?;
        let abs = page.first_row + row as u64;
        if let Some(Full::Loaded(v)) = self.values.full.get(&(abs, column)) {
            return Some(Cow::Borrowed(v.as_ref()));
        }
        page.rows.get(row)?.get(column).map(Cow::Borrowed)
    }

    pub(super) fn display_type(&self, column: usize) -> DisplayType {
        self.page
            .as_ref()
            .and_then(|p| p.columns.get(column))
            .and_then(|c| self.values.display.get(c.name.as_ref()))
            .copied()
            .unwrap_or_default()
    }

    // ---- panel ------------------------------------------------------------------

    /// A column header's label: a binary column says how it shows its
    /// values (`blob (UUID)`).
    pub(super) fn header_label(&self, column: usize) -> gpui::SharedString {
        let Some(c) = self.page.as_ref().and_then(|p| p.columns.get(column)) else {
            return gpui::SharedString::default();
        };
        if self.is_transposed() || c.kind != db_core::ValueKind::Bytes {
            return c.name.clone();
        }
        let shown = match self.display_type(column) {
            DisplayType::Detect => {
                let first = self.page.as_ref().and_then(|p| {
                    p.rows
                        .iter()
                        .find_map(|r| r.get(column).and_then(value::bytes_of))
                });
                match first {
                    Some(b) if b.len() == 16 => "UUID",
                    _ => "Hex",
                }
            }
            DisplayType::Uuid | DisplayType::UuidSwapped => "UUID",
            DisplayType::Text => "Text",
            DisplayType::Hex => "Hex",
            DisplayType::HexAscii => "Hex and ASCII",
        };
        format!("{} ({shown})", c.name).into()
    }

    pub(super) fn open_panel(
        &mut self,
        tab: PanelTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.values.view == ViewMode::Text {
            self.set_view(ViewMode::Table, window, cx);
        }
        match &mut self.values.panel {
            Some(panel) => panel.tab = tab,
            None => {
                self.values.panel = Some(Panel::new(tab, window, cx));
            }
        }
        if self.selection.is_none() {
            self.select(Selection::cell((0, 0)), cx);
        }
        cx.notify();
    }

    pub(super) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.values.panel.take().is_some() {
            window.focus(&self.focus);
            cx.notify();
        }
    }

    pub(super) fn toggle_json(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.values.panel {
            p.json = !p.json;
            cx.notify();
        }
    }

    pub(super) fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.values.panel {
            p.wrap = !p.wrap;
            let wrap = p.wrap;
            p.editor.update(cx, |e, cx| e.set_soft_wraps(wrap, cx));
            cx.notify();
        }
    }

    /// Makes the panel's editor show the current cell's value. Called as
    /// the grid draws, so it follows the selection, the page, the display
    /// types and a value that was read.
    pub(super) fn sync_panel(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = &self.values.panel else {
            return;
        };
        if panel.tab != PanelTab::Value {
            return;
        }
        let Some(page) = self.page.clone() else {
            return;
        };
        let pos = self.data_pos();
        let (text, json) = match pos.and_then(|p| self.cell_value(p).map(|v| (p, v))) {
            None => (String::new(), false),
            Some((pos, v)) => {
                let formats = settings_ui::store::get::<DataFormats>(cx);
                let mut text =
                    value::panel_text(&v, &formats, self.display_type(pos.1)).unwrap_or_default();
                let json = value::is_json(&v);
                if json
                    && panel.json
                    && let Some(pretty) = value::pretty_json(&text)
                {
                    text = pretty;
                }
                (text, json)
            }
        };
        let key = (
            Arc::as_ptr(&page) as usize,
            pos,
            text.len(),
            panel.json,
            json,
        );
        let panel = self.values.panel.as_mut().expect("the panel is open");
        panel.is_json = json;
        if panel.shown == Some(key) {
            return;
        }
        panel.shown = Some(key);
        let editor = panel.editor.clone();
        editor.update(cx, |e, cx| {
            e.set_text_selected(&text, cx);
            // The reference selects nothing; a caret at the start.
            e.set_selections(SelectionSet::single(TextSelection::new(0, 0)), cx);
        });
    }

    // ---- views ------------------------------------------------------------------

    pub(super) fn set_view(&mut self, view: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let old = self.values.view;
        if old == view {
            return;
        }
        // The current cell stays the current cell.
        let data = self.data_pos();
        self.values.view = view;
        self.values.transposed = None;
        if view == ViewMode::Transpose {
            self.values.transposed = self.page.as_ref().map(|p| Arc::new(views::transposed(p)));
        }
        if let Some(pos) = data {
            self.selection = None;
            self.select_data_cell(pos, cx);
        } else {
            self.selection = None;
        }
        match view {
            ViewMode::Text => {
                self.values.panel = None;
                let editor = self.text_editor(window, cx);
                window.focus(&editor.focus_handle(cx));
            }
            _ => window.focus(&self.focus),
        }
        status::changed(cx);
        cx.notify();
    }

    fn text_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        if let Some(t) = &self.values.text {
            return t.editor.clone();
        }
        let editor = cx.new(|cx| {
            let mut buffer = text::Buffer::new("");
            buffer.set_read_only(true);
            let mut e = Editor::new(buffer, window, cx);
            e.set_viewer(true);
            e.set_soft_wraps(false, cx);
            e
        });
        self.values.text = Some(TextView {
            editor: editor.clone(),
            made_for: None,
        });
        editor
    }

    /// Makes the text view's editor hold the page's CSV; called as the grid
    /// draws.
    pub(super) fn sync_text_view(&mut self, cx: &mut Context<Self>) {
        if self.values.view != ViewMode::Text {
            return;
        }
        let Some(page) = self.page.clone() else {
            return;
        };
        let mut hidden: Vec<&str> = self.hidden.iter().map(|n| n.as_ref()).collect();
        hidden.sort_unstable();
        let hash = hidden.iter().fold(7u64, |h, n| {
            n.bytes().fold(h.wrapping_mul(31), |h, b| {
                h.wrapping_mul(131).wrapping_add(u64::from(b))
            })
        });
        let key = (Arc::as_ptr(&page) as usize, hash);
        let Some(view) = &mut self.values.text else {
            return;
        };
        if view.made_for == Some(key) {
            return;
        }
        view.made_for = Some(key);
        let editor = view.editor.clone();
        let text = views::csv_text(&page, &|c| !self.is_hidden(c));
        editor.update(cx, |e, cx| {
            e.set_text_selected(&text, cx);
            e.set_selections(SelectionSet::single(TextSelection::new(0, 0)), cx);
        });
    }

    // ---- display types and formats ----------------------------------------------

    pub(super) fn set_display_type(
        &mut self,
        column: usize,
        display: DisplayType,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let Some(name) = self
            .page
            .as_ref()
            .and_then(|p| p.columns.get(column))
            .map(|c| c.name.to_string())
        else {
            return;
        };
        if display == DisplayType::default() {
            self.values.display.remove(&name);
        } else {
            self.values.display.insert(name, display);
        }
        self.reformat(cx);
    }

    /// Makes the cell texts again (other formats or display types) without
    /// reading the page again.
    pub(super) fn reformat(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.clone() else {
            return;
        };
        let fmt = self.cell_format(cx);
        let build = cx.background_spawn({
            let page = page.clone();
            async move { page.reformatted(&fmt) }
        });
        self.values.reformat = Some(cx.spawn(async move |this, cx| {
            let made = build.await;
            let _ = this.update(cx, |g, cx| {
                // Only while the page is the one it was made from.
                if g.page.as_ref().is_some_and(|p| Arc::ptr_eq(p, &page)) {
                    g.page = Some(Arc::new(made));
                    if g.values.view == ViewMode::Transpose {
                        g.values.transposed =
                            g.page.as_ref().map(|p| Arc::new(views::transposed(p)));
                    }
                    cx.notify();
                }
            });
        }));
    }

    /// The page changed: what was made for the old one goes.
    pub(super) fn values_page_changed(&mut self) {
        self.values.full.clear();
        self.values.transposed = (self.values.view == ViewMode::Transpose)
            .then(|| self.page.as_ref().map(|p| Arc::new(views::transposed(p))))
            .flatten();
    }

    // ---- the whole value --------------------------------------------------------

    /// The row's key for a keyed read: the primary key's values, or, with
    /// none, every column whose value can be compared as a literal.
    fn key_of(&self, row: usize) -> Result<Vec<(String, DbValue)>, String> {
        let page = self.page.as_ref().ok_or("there is no page")?;
        let values = page.rows.get(row).ok_or("the row is gone")?;
        let primary: Vec<usize> = page
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.key == KeyMark::Primary)
            .map(|(i, _)| i)
            .collect();
        let columns: Vec<usize> = if primary.is_empty() {
            (0..page.column_count())
                .filter(|i| values.get(*i).is_some_and(db_session::is_keyable))
                .collect()
        } else {
            primary
        };
        let mut key = Vec::new();
        for i in columns {
            let v = values.get(i).cloned().unwrap_or(DbValue::Null);
            if !db_session::is_keyable(&v) {
                return Err(format!(
                    "the key column {} has a value that cannot be searched for",
                    page.columns[i].name
                ));
            }
            key.push((page.columns[i].name.to_string(), v));
        }
        if key.is_empty() {
            return Err("the row has no key to find it by".to_owned());
        }
        Ok(key)
    }

    /// Reads the whole value of a data cell; with `save`, writes it to the
    /// file then.
    pub(super) fn load_full(
        &mut self,
        (row, column): Pos,
        save: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page.clone() else {
            return;
        };
        let abs = page.first_row + row as u64;
        if matches!(
            self.values.full.get(&(abs, column)),
            Some(Full::Loading { .. })
        ) {
            return;
        }
        let key = match self.key_of(row) {
            Ok(k) => k,
            Err(e) => {
                self.values.full.insert((abs, column), Full::Failed(e));
                cx.notify();
                return;
            }
        };
        let query = CellQuery {
            table: self.table.clone(),
            column: page.columns[column].name.to_string(),
            key,
        };
        let job = self.pager.cell(&query);
        self.values
            .full
            .insert((abs, column), Full::Loading { job, save });
        cx.notify();
    }

    pub(super) fn on_cell(
        &mut self,
        job: JobId,
        result: Result<DbValue, PageError>,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = self.values.full.iter().find_map(|(k, f)| match f {
            Full::Loading { job: j, .. } if *j == job => Some(*k),
            _ => None,
        }) else {
            return;
        };
        let Some(Full::Loading { save, .. }) = self.values.full.remove(&cell) else {
            return;
        };
        match result {
            Ok(value) => {
                if let Some(path) = save {
                    self.write_value(path, &value, cx);
                }
                self.values.full.insert(cell, Full::Loaded(Arc::new(value)));
            }
            Err(e) if e.is_cancelled() => {}
            Err(e) => {
                self.values
                    .full
                    .insert(cell, Full::Failed(e.error.message.clone()));
            }
        }
        cx.notify();
    }

    /// Stops the reads of whole values.
    pub(super) fn cancel_full(&mut self) {
        let jobs: Vec<(u64, usize)> = self
            .values
            .full
            .iter()
            .filter(|(_, f)| matches!(f, Full::Loading { .. }))
            .map(|(k, _)| *k)
            .collect();
        for k in jobs {
            if let Some(Full::Loading { job, .. }) = self.values.full.remove(&k) {
                self.pager.cancel(job);
            }
        }
    }

    /// Saves the current cell's value: asks for a file unless `path` is
    /// given; a cut value is read whole first.
    pub(super) fn save_value(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let Some(pos) = self.data_pos() else {
            return;
        };
        let Some(v) = self.cell_value(pos) else {
            return;
        };
        if v.is_null() {
            return;
        }
        match path {
            Some(path) => self.save_value_to(pos, PathBuf::from(path), cx),
            None => {
                let name = self
                    .page
                    .as_ref()
                    .and_then(|p| p.columns.get(pos.1))
                    .map_or("value".to_owned(), |c| c.name.to_string());
                let suggested = if value::bytes_of(&v).is_some() {
                    format!("{name}.bin")
                } else {
                    format!("{name}.txt")
                };
                let dir = std::env::current_dir().unwrap_or_default();
                let chosen = cx.prompt_for_new_path(&dir, Some(&suggested));
                cx.spawn(async move |this, cx| {
                    if let Ok(Ok(Some(path))) = chosen.await {
                        let _ = this.update(cx, |g, cx| g.save_value_to(pos, path, cx));
                    }
                })
                .detach();
            }
        }
    }

    fn save_value_to(&mut self, pos: Pos, path: PathBuf, cx: &mut Context<Self>) {
        let Some(v) = self.cell_value(pos) else {
            return;
        };
        if value::is_truncated(&v) {
            // The whole value first; it is written when it arrives.
            drop(v);
            self.load_full(pos, Some(path), cx);
        } else {
            let v = v.into_owned();
            self.write_value(path, &v, cx);
        }
    }

    /// Writes a value off the UI thread: bytes as they are, text as UTF-8.
    fn write_value(&mut self, path: PathBuf, v: &DbValue, cx: &mut Context<Self>) {
        let bytes = value_bytes(v);
        let write = cx.background_spawn(async move { std::fs::write(&path, bytes).map(|_| path) });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |g, cx| {
                g.values.saved = Some(match result {
                    Ok(path) => Ok(path.display().to_string()),
                    Err(e) => Err(e.to_string()),
                });
                cx.notify();
            });
        })
        .detach();
    }

    // ---- aggregates -------------------------------------------------------------

    pub(super) fn aggregates(&self) -> Option<aggregate::Aggregates> {
        let page = self.shown_page()?;
        let selection = self.selection?;
        Some(aggregate::aggregates(page, &selection, &|c| {
            !self.is_hidden(c)
        }))
    }

    pub(super) fn copy_aggregation(&mut self, name: Option<&str>, cx: &mut Context<Self>) {
        let Some(a) = self.aggregates() else {
            return;
        };
        if let Some(text) = a.get(name.unwrap_or("SUM")) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // ---- state ------------------------------------------------------------------

    pub(super) fn values_state_json(&self, cx: &App) -> Json {
        let cell = self
            .data_pos()
            .and_then(|p| self.cell_value(p).map(|v| (p, v)));
        let full = self.data_pos().and_then(|(r, c)| {
            let abs = self.page.as_ref()?.first_row + r as u64;
            self.values.full.get(&(abs, c)).map(|f| match f {
                Full::Loading { .. } => json!("loading"),
                Full::Loaded(v) => json!({"loaded": value::panel_text(v, &DataFormats::default(), DisplayType::Hex).map(|t| t.len())}),
                Full::Failed(e) => json!({"error": e}),
            })
        });
        json!({
            "view": self.values.view.id(),
            "transposed": self.values.transposed.as_ref().map(|p| json!({
                "columns": p.columns.iter().map(|c| c.name.as_ref()).collect::<Vec<_>>(),
                "rows": p.row_count(),
                "cells": p.cells.iter().take(20).map(|r| r.iter()
                    .map(|c| c.as_ref().map_or(Json::Null, |t| json!(t.as_ref())))
                    .collect::<Vec<_>>()).collect::<Vec<_>>(),
            })),
            "text": self.values.text.as_ref().filter(|_| self.values.view == ViewMode::Text)
                .map(|t| t.editor.read(cx).text()),
            "display": self.values.display.iter()
                .map(|(k, v)| (k.clone(), json!(v.id())))
                .collect::<serde_json::Map<_, _>>(),
            "panel": self.values.panel.as_ref().map(|p| p.state_json(cx)),
            "value": cell.map(|((r, c), v)| json!({
                "row": r,
                "column": c,
                "kind": v.kind().map(|k| format!("{k:?}")),
                "null": v.is_null(),
                "truncated": value::is_truncated(&v),
                "full_len": match v.as_ref() {
                    DbValue::Truncated { full_len, .. } => *full_len,
                    _ => None,
                },
                "full": full,
            })),
            "aggregates": self.aggregates().map(|a| a.lines().into_iter()
                .map(|(k, v)| (k.to_owned(), json!(v)))
                .collect::<serde_json::Map<_, _>>()),
            "saved": self.values.saved,
        })
    }
}

/// The bytes a value is saved as.
pub fn value_bytes(v: &DbValue) -> Vec<u8> {
    match v {
        DbValue::Bytes(b) => b.clone(),
        DbValue::Truncated { prefix, .. } => value_bytes(prefix),
        other => value::panel_text(other, &DataFormats::default(), DisplayType::Hex)
            .unwrap_or_default()
            .into_bytes(),
    }
}

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    settings_ui::store::register::<DataFormats>(cx);
    // Open grids make their texts again when the formats change.
    settings_ui::store::observe(cx, formats::KEY, |cx| {
        let grids: Vec<_> = cx
            .try_global::<Grids>()
            .map(|g| g.0.iter().filter_map(|(_, w)| w.upgrade()).collect())
            .unwrap_or_default();
        for grid in grids {
            grid.update(cx, |g, cx| g.reformat(cx));
        }
    });
    macro_rules! on {
        ($action:ty, |$g:ident, $a:ident, $w:ident, $cx:ident| $body:expr) => {
            cx.on_action(|a: &$action, cx| {
                let $a = a.clone();
                with_target(cx, move |$g, $w, $cx| {
                    let _ = (&$a, &$w);
                    $body
                })
            });
        };
    }
    on!(ShowValue, |g, a, w, cx| g.open_panel(
        PanelTab::Value,
        w,
        cx
    ));
    on!(ShowRecord, |g, a, w, cx| g.open_panel(
        PanelTab::Record,
        w,
        cx
    ));
    on!(ShowAggregates, |g, a, w, cx| g.open_panel(
        PanelTab::Aggregates,
        w,
        cx
    ));
    on!(HidePanel, |g, a, w, cx| g.close_panel(w, cx));
    on!(SetView, |g, a, w, cx| {
        if let Some(view) = ViewMode::from_id(&a.view) {
            g.set_view(view, w, cx)
        }
    });
    on!(SetDisplayType, |g, a, w, cx| {
        if let Some(d) = DisplayType::from_id(&a.display) {
            g.set_display_type(a.column, d, cx)
        }
    });
    on!(LoadFullValue, |g, a, w, cx| {
        if let Some(pos) = g.data_pos() {
            g.load_full(pos, None, cx)
        }
    });
    on!(SaveValue, |g, a, w, cx| g.save_value(a.path.clone(), cx));
    on!(ToggleJson, |g, a, w, cx| g.toggle_json(cx));
    on!(ToggleWrap, |g, a, w, cx| g.toggle_wrap(cx));
    on!(CopyAggregation, |g, a, w, cx| g
        .copy_aggregation(a.name.as_deref(), cx));
}
