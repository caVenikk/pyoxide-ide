//! The Import dialog (shots DB80 to DB83): a delimited text file into an
//! existing table. The file's format (delimiter, encoding, header) is found
//! from its start and can be changed, every table column takes its values
//! from a file column (paired by name, or by position without a header), and
//! the first rows are previewed as the table will get them. OK closes the
//! dialog and runs the import in the background ([`super::run`]): one
//! transaction, so a row that does not fit leaves the table unchanged.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use db_session::import::{self, ColumnMap, ImportFormat, Preview};
use db_session::{Encoding, ImportRequest};
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, FocusHandle,
    Focusable, IntoElement, PathPromptOptions, Pixels, Render, SharedString, Task, Window, canvas,
    div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{self, BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::menu::MenuItem;
use ui::theme::ThemeExt;

use super::run;
use super::{
    ImportBrowse, ImportCancel, ImportMap, ImportMapMenu, ImportOk, ImportSetDelimiter,
    ImportSetFile, ImportSetOption, TransferTarget,
};

const TITLE: &str = "Import";
const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 640.0;
const LABEL: f32 = 80.0;
const KEY_CONTEXT: &str = "DatabaseImportDialog";
const PREVIEW_ROWS: usize = 8;
/// Widths of the mapping table's columns.
const MAP_COLUMN: f32 = 240.0;
const MAP_TYPE: f32 = 200.0;
const MAP_SOURCE: f32 = 300.0;
const PREVIEW_COLUMN: f32 = 130.0;

/// The delimiters of the list: their names and bytes.
const DELIMITERS: [(&str, &str, u8); 4] = [
    ("comma", "Comma (,)", b','),
    ("semicolon", "Semicolon (;)", b';'),
    ("pipe", "Pipe (|)", b'|'),
    ("tab", "Tab", b'\t'),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ListKind {
    Delimiter,
    Column(usize),
}

pub(crate) struct ImportDialog {
    target: TransferTarget,
    parent: AnyWindowHandle,
    file: Option<PathBuf>,
    format: ImportFormat,
    encoding: Option<Encoding>,
    inconvertible_as_null: bool,
    preview: Option<Preview>,
    error: Option<String>,
    /// One entry per table column.
    mapping: Vec<ColumnMap>,
    /// The mapping the file's header gives, to tell a choice from it.
    auto: Vec<ColumnMap>,
    list: Option<(ListKind, MenuPopup)>,
    anchors: std::rc::Rc<RefCell<HashMap<ListKind, Bounds<Pixels>>>>,
    reading: Task<()>,
    focus: FocusHandle,
}

impl Focusable for ImportDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Opens the dialog over the main window `window`: for `path`, or, without
/// one, for the file the chooser returns.
pub(crate) fn open(
    target: TransferTarget,
    path: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let parent = window.window_handle();
    match path {
        Some(path) => open_dialog(target, Some(PathBuf::from(path)), window, cx),
        None => {
            let chosen = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            });
            cx.spawn(async move |cx| {
                let Ok(Ok(Some(paths))) = chosen.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let _ = cx.update(|cx| {
                    let _ = parent.update(cx, |_, window, cx| {
                        open_dialog(target, Some(path), window, cx)
                    });
                });
            })
            .detach();
        }
    }
}

fn open_dialog(target: TransferTarget, path: Option<PathBuf>, window: &mut Window, cx: &mut App) {
    let options = ui::dialog_window::DialogOptions {
        title: TITLE.into(),
        size: gpui::size(
            px(WIDTH),
            px(HEIGHT + ui::dialog_window::title_height(true)),
        ),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let parent = window.window_handle();
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| ImportDialog::new(target, parent, path, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        super::set_open_import(Some(dialog.downgrade()), cx);
    }
}

fn delimiter_id(byte: u8) -> &'static str {
    DELIMITERS
        .iter()
        .find(|(_, _, b)| *b == byte)
        .map_or("comma", |(id, _, _)| id)
}

impl ImportDialog {
    fn new(
        target: TransferTarget,
        parent: AnyWindowHandle,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut dialog = Self {
            target,
            parent,
            file: None,
            format: ImportFormat::default(),
            encoding: None,
            inconvertible_as_null: false,
            preview: None,
            error: None,
            mapping: Vec::new(),
            auto: Vec::new(),
            list: None,
            anchors: Default::default(),
            reading: Task::ready(()),
            focus,
        };
        if let Some(path) = path {
            dialog.set_file(path, cx);
        }
        dialog
    }

    fn table_columns(&self) -> Vec<String> {
        self.target
            .target
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    // ---- reading the file --------------------------------------------------------

    /// Takes `path`: its format is found from its start, then its first
    /// rows are read.
    fn set_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.file = Some(path.clone());
        self.error = None;
        self.preview = None;
        let read = cx.background_spawn(async move {
            let (format, encoding) = import::detect_format(&path)?;
            let preview = import::preview(&path, &format, Some(encoding), PREVIEW_ROWS)?;
            Ok::<_, std::io::Error>((format, encoding, preview))
        });
        self.reading = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |d, cx| {
                match result {
                    Ok((format, encoding, preview)) => {
                        d.format = format;
                        d.encoding = Some(encoding);
                        d.set_preview(preview);
                    }
                    Err(e) => d.error = Some(format!("Cannot read the file: {e}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Reads the first rows again with the options changed.
    fn reread(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.file.clone() else {
            return;
        };
        let (format, encoding) = (self.format.clone(), self.encoding);
        let read =
            cx.background_spawn(
                async move { import::preview(&path, &format, encoding, PREVIEW_ROWS) },
            );
        self.reading = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |d, cx| {
                match result {
                    Ok(preview) => {
                        d.error = None;
                        d.set_preview(preview);
                    }
                    Err(e) => d.error = Some(format!("Cannot read the file: {e}")),
                }
                cx.notify();
            });
        });
    }

    fn set_preview(&mut self, preview: Preview) {
        self.error = None;
        self.auto = import::auto_map(&self.table_columns(), &preview.header, self.format.header);
        self.mapping = self.auto.clone();
        self.preview = Some(preview);
    }

    // ---- choices -----------------------------------------------------------------

    fn set_option(&mut self, option: &str, value: bool, cx: &mut Context<Self>) {
        match option {
            "header" => {
                self.format.header = value;
                self.reread(cx);
            }
            "empty-as-null" => {
                self.format.empty_as_null = value;
                self.reread(cx);
            }
            "inconvertible-as-null" => self.inconvertible_as_null = value,
            _ => return,
        }
        cx.notify();
    }

    fn set_delimiter(&mut self, name: &str, cx: &mut Context<Self>) {
        self.close_list(cx);
        let byte = DELIMITERS
            .iter()
            .find(|(id, _, _)| *id == name)
            .map(|(_, _, b)| *b)
            .or_else(|| name.bytes().next().filter(|_| name.len() == 1));
        if let Some(byte) = byte {
            self.format.csv.delimiter = byte;
            self.reread(cx);
            cx.notify();
        }
    }

    fn map(&mut self, column: &str, source: Option<usize>, cx: &mut Context<Self>) {
        self.close_list(cx);
        if let Some(m) = self.mapping.iter_mut().find(|m| m.column == column) {
            m.source = source;
        }
        cx.notify();
    }

    fn file_columns(&self) -> &[String] {
        self.preview.as_ref().map_or(&[], |p| &p.header)
    }

    /// What a mapping row shows for its choice.
    fn source_label(&self, i: usize) -> String {
        let (Some(m), Some(a)) = (self.mapping.get(i), self.auto.get(i)) else {
            return String::new();
        };
        let name = |s: usize| {
            self.file_columns()
                .get(s)
                .cloned()
                .unwrap_or_else(|| format!("Column{}", s + 1))
        };
        match (m.source, a.source) {
            (Some(s), Some(auto)) if s == auto => format!("<Auto> ({})", name(s)),
            (Some(s), _) => name(s),
            (None, _) => "<Not mapped>".to_owned(),
        }
    }

    fn can_import(&self) -> bool {
        self.file.is_some()
            && self.preview.is_some()
            && self.mapping.iter().any(|m| m.source.is_some())
    }

    // ---- running -----------------------------------------------------------------

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        super::set_open_import(None, cx);
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_import() {
            return;
        }
        let Some(file) = self.file.clone() else {
            return;
        };
        let request = ImportRequest {
            table: self.target.target.table.clone(),
            file,
            format: self.format.clone(),
            encoding: self.encoding,
            mapping: self
                .mapping
                .iter()
                .filter(|m| m.source.is_some())
                .cloned()
                .collect(),
            inconvertible_as_null: self.inconvertible_as_null,
        };
        let transfer = self.target.clone();
        let parent = self.parent;
        self.close(window, cx);
        cx.defer(move |cx| run::import(&transfer, request, parent, cx));
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                let _ = this.update(cx, |d, cx| d.set_file(path, cx));
            }
        })
        .detach();
    }

    // ---- lists -------------------------------------------------------------------

    fn open_list(&mut self, kind: ListKind, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        let items: Vec<MenuItem> = match kind {
            ListKind::Delimiter => DELIMITERS
                .iter()
                .map(|(id, label, byte)| {
                    MenuItem::action(
                        *label,
                        ImportSetDelimiter {
                            delimiter: (*id).to_owned(),
                        },
                    )
                    .bold(*byte == self.format.csv.delimiter)
                })
                .collect(),
            ListKind::Column(i) => {
                let Some(column) = self.mapping.get(i).map(|m| m.column.clone()) else {
                    return;
                };
                let current = self.mapping[i].source;
                let mut items: Vec<MenuItem> = self
                    .file_columns()
                    .iter()
                    .enumerate()
                    .map(|(s, name)| {
                        MenuItem::action(
                            name.clone(),
                            ImportMap {
                                column: column.clone(),
                                source: Some(s),
                            },
                        )
                        .bold(current == Some(s))
                    })
                    .collect();
                items.push(MenuItem::separator());
                items.push(MenuItem::action(
                    "<Not mapped>",
                    ImportMap {
                        column,
                        source: None,
                    },
                ));
                items
            }
        };
        let bounds = self.anchors.borrow().get(&kind).copied();
        let position = match bounds {
            Some(b) => point(b.left(), b.bottom() + px(2.0)),
            None => point(px(40.0), px(80.0)),
        };
        let activate = !workbench::env::background();
        let popup = MenuPopup::open(
            items,
            None,
            position,
            &self.focus,
            activate,
            window,
            cx,
            |this, cx| this.close_list(cx),
        );
        self.list = popup.map(|p| (kind, p));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if let Some((_, popup)) = self.list.take() {
            popup.close(cx);
        }
    }

    pub(crate) fn state_json(&self, _: &App) -> Json {
        let preview = self.preview.as_ref();
        json!({
            "table": self.target.title(),
            "file": self.file.as_ref().map(|f| f.display().to_string()),
            "delimiter": delimiter_id(self.format.csv.delimiter),
            "encoding": self.encoding.map(|e| e.title()),
            "header": self.format.header,
            "empty_as_null": self.format.empty_as_null,
            "inconvertible_as_null": self.inconvertible_as_null,
            "file_columns": preview.map(|p| p.header.clone()),
            "mapping": self.mapping.iter().enumerate().map(|(i, m)| json!({
                "column": m.column,
                "source": m.source,
                "label": self.source_label(i),
            })).collect::<Vec<_>>(),
            "preview_rows": preview.map(|p| p.rows.clone()),
            "error": self.error,
            "can_import": self.can_import(),
            "list_open": self.list.is_some(),
        })
    }

    // ---- drawing -----------------------------------------------------------------

    fn check(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        form::check_row(
            SharedString::from(format!("import-{id}")),
            SharedString::from(format!("import-{id}-box")),
            on,
            div().child(label),
            cx,
        )
        .debug_selector(move || format!("import-{id}"))
        .h(px(form::FIELD_HEIGHT))
        .on_click(cx.listener(move |this, _, _, cx| this.set_option(id, !on, cx)))
        .into_any_element()
    }

    fn list_combo(
        &self,
        id: SharedString,
        kind: ListKind,
        text: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let anchors = self.anchors.clone();
        let selector = id.to_string();
        form::combo(id, vec![div().child(text).into_any_element()], cx)
            .debug_selector(move || selector.clone())
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(kind, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_list(kind, window, cx)
                }),
            )
            .into_any_element()
    }

    fn render_mapping(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let line = theme.color(UiRole::Divider);
        let muted = theme.color(UiRole::TextMuted);
        let head = |text: &'static str, width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .px(px(8.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .text_color(muted)
                .child(text)
        };
        let mut rows = div().flex().flex_col();
        for (i, column) in self.target.target.columns.iter().enumerate() {
            let combo = self.list_combo(
                SharedString::from(format!("import-map-{}", column.name)),
                ListKind::Column(i),
                self.source_label(i),
                cx,
            );
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(30.0))
                    .child(
                        div()
                            .w(px(MAP_COLUMN))
                            .flex_none()
                            .px(px(8.0))
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(MAP_TYPE))
                            .flex_none()
                            .px(px(8.0))
                            .text_color(muted)
                            .child(column.data_type.clone()),
                    )
                    .child(div().w(px(MAP_SOURCE)).flex_none().child(combo)),
            );
        }
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(line)
            .rounded(form::field_radius(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .border_b_1()
                    .border_color(line)
                    .child(head("to: Column", MAP_COLUMN))
                    .child(head("to: Type", MAP_TYPE))
                    .child(head("from: Column", MAP_SOURCE)),
            )
            .child(rows)
            .into_any_element()
    }

    fn render_preview(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let line = theme.color(UiRole::Divider);
        let muted = theme.color(UiRole::TextMuted);
        let Some(preview) = &self.preview else {
            return div().into_any_element();
        };
        let mapped: Vec<(&str, usize)> = self
            .mapping
            .iter()
            .filter_map(|m| Some((m.column.as_str(), m.source?)))
            .collect();
        let cell = |text: AnyElement| {
            div()
                .w(px(PREVIEW_COLUMN))
                .flex_none()
                .h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .border_r_1()
                .border_color(line)
                .child(text)
        };
        let mut table = div().flex().flex_col().border_1().border_color(line);
        table = table.child(
            div()
                .flex()
                .flex_row()
                .border_b_1()
                .border_color(line)
                .children(mapped.iter().map(|(name, _)| {
                    cell(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(name.to_string())
                            .into_any_element(),
                    )
                })),
        );
        for row in &preview.rows {
            table = table.child(
                div()
                    .flex()
                    .flex_row()
                    .children(mapped.iter().map(|(_, s)| {
                        let text = match row.get(*s) {
                            Some(Some(t)) => div()
                                .child(t.lines().next().unwrap_or("").to_owned())
                                .into_any_element(),
                            _ => div().text_color(muted).child("<null>").into_any_element(),
                        };
                        cell(text)
                    })),
            );
        }
        let file = self
            .file
            .as_ref()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().text_color(muted).child(format!(
                "file: {file}   format: CSV   encoding: {}",
                self.encoding.map_or("", |e| e.title())
            )))
            .child(div().overflow_hidden().child(table))
            .into_any_element()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let danger = theme.color(UiRole::Danger);
        let (surface, text_color) = (theme.color(UiRole::Surface), theme.color(UiRole::Text));
        let (ui_size, ui_family) = (theme.ui_size(), theme.ui_family());
        let file_text = self
            .file
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_default();
        let file = ui::components::input_frame(false, false, cx)
            .h(px(form::FIELD_HEIGHT))
            .px(px(8.0))
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(file_text)
            .child(div().flex_1())
            .child(form::browse_button(
                "import-browse",
                Some(Box::new(ImportBrowse)),
            ));
        let row = |label: &'static str, value: AnyElement| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(LABEL)).flex_none().child(label))
                .child(value)
        };
        let delimiter = self.list_combo(
            "import-delimiter".into(),
            ListKind::Delimiter,
            DELIMITERS
                .iter()
                .find(|(_, _, b)| *b == self.format.csv.delimiter)
                .map_or_else(
                    || (self.format.csv.delimiter as char).to_string(),
                    |(_, l, _)| (*l).to_owned(),
                ),
            cx,
        );
        let bar = BottomBar::new()
            .button(BarButton::new(
                "import-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(ImportCancel),
            ))
            .button(
                BarButton::new("import-ok", "OK", ButtonRole::Ok, Box::new(ImportOk))
                    .enabled(self.can_import()),
            )
            .render(cx);
        let body = div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .child(row("File:", file.into_any_element()))
            .child(row(
                "Table:",
                div()
                    .child(format!(
                        "{} [{}]",
                        self.target.title(),
                        self.target.target.source
                    ))
                    .into_any_element(),
            ))
            .child(row(
                "Delimiter:",
                div().w(px(200.0)).child(delimiter).into_any_element(),
            ))
            .child(div().text_color(muted).child("Mapping"))
            .child(self.render_mapping(cx))
            .child(self.check("header", "First row is a header", self.format.header, cx))
            .child(self.check(
                "empty-as-null",
                "Insert empty values as null",
                self.format.empty_as_null,
                cx,
            ))
            .child(self.check(
                "inconvertible-as-null",
                "Insert inconvertible values as null",
                self.inconvertible_as_null,
                cx,
            ))
            .child(div().text_color(muted).child("Data Preview"))
            .child(self.render_preview(cx))
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(danger).child(e)),
            );
        div()
            .id("import-dialog")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, a: &ImportSetFile, _, cx| {
                this.set_file(PathBuf::from(&a.path), cx)
            }))
            .on_action(cx.listener(|this, _: &ImportBrowse, window, cx| this.browse(window, cx)))
            .on_action(cx.listener(|this, a: &ImportSetOption, _, cx| {
                this.set_option(&a.option, a.value, cx)
            }))
            .on_action(cx.listener(|this, a: &ImportSetDelimiter, _, cx| {
                this.set_delimiter(&a.delimiter, cx)
            }))
            .on_action(cx.listener(|this, a: &ImportMap, _, cx| this.map(&a.column, a.source, cx)))
            .on_action(cx.listener(|this, a: &ImportMapMenu, window, cx| {
                if let Some(i) = this.mapping.iter().position(|m| m.column == a.column) {
                    this.open_list(ListKind::Column(i), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ImportOk, window, cx| this.ok(window, cx)))
            .on_action(cx.listener(|this, _: &ImportCancel, window, cx| this.close(window, cx)))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "enter" => this.ok(window, cx),
                    "escape" => this.close(window, cx),
                    _ => {}
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(surface)
            .text_color(text_color)
            .text_size(ui_size)
            .font_family(ui_family)
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
            .child(body)
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
