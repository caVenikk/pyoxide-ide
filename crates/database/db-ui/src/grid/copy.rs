//! Copying rows out of the grid and the toolbar's transfer buttons (shot
//! DB48): the selection is written in the data extractor of the toolbar
//! (CSV at first) and put on the clipboard, or in another one chosen for
//! this copy ("Copy As"); the whole table goes to the clipboard through an
//! export job. The text is made from the values of the shown page, so a copy
//! is what the export writes for the same rows. The Export Data and Import
//! dialogs open from here for the grid's table.

use db_core::{Column, Row, Value};
use db_session::export::writers;
use db_session::{ExportOptions, ExportTarget, Format, TableRef};
use gpui::{
    AnyElement, App, ClipboardItem, Context, DismissEvent, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Point, Window, div, point, prelude::*, px,
};
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::{OpenMenu, TableGrid, target, with_target};
use crate::icons;
use crate::transfer::{self, ExportData, ImportData, TransferTarget};
use themes::UiRole;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Copy the selection.
    CopyData => "db.grid.copy", "Copy Using Data Extractor",
        "Puts the selected cells of the active table grid on the clipboard, written by its data extractor (the toolbar's, CSV at first).";
    /// Copy the selection in a given format.
    CopyAs { format: String } => "db.grid.copy-as", "Copy As",
        "Puts the selected cells of the active table grid on the clipboard in extractor `format` (`csv`, `tsv`, `pipe-separated`, `semicolon-separated`, `html`, `json`, `markdown`, `sql-inserts`, `sql-updates`, `where-clause`).",
        r#"{"format": "json"}"#;
    /// Choose the toolbar's extractor.
    SetExtractor { format: String } => "db.grid.set-extractor", "Change Data Extractor",
        "Makes extractor `format` the one that Copy of the active table grid uses.",
        r#"{"format": "tsv"}"#;
    /// Show the extractor list.
    ExtractorMenu => "db.grid.extractor-menu", "Data Extractor",
        "Shows the list of data extractors under the toolbar's extractor button of the active table grid.";
    /// Copy the whole table.
    ExportTableToClipboard => "db.grid.export-table-to-clipboard", "Export Table to Clipboard",
        "Copies every row of the active table grid's table (with its WHERE and ORDER BY) to the clipboard in its data extractor, as a background task.";
}

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &CopyData, cx| with_target(cx, |g, _, cx| g.copy_selection(None, cx)));
    cx.on_action(|a: &CopyAs, cx| {
        let format = Format::from_id(&a.format);
        with_target(cx, move |g, _, cx| {
            if let Some(format) = format {
                g.copy_selection(Some(format), cx);
            }
        })
    });
    cx.on_action(|a: &SetExtractor, cx| {
        let format = Format::from_id(&a.format);
        with_target(cx, move |g, _, cx| {
            if let Some(format) = format {
                g.set_extractor(format, cx);
            }
        })
    });
    cx.on_action(|_: &ExtractorMenu, cx| {
        with_target(cx, |g, window, cx| g.show_extractor_menu(None, window, cx))
    });
    cx.on_action(|_: &ExportTableToClipboard, cx| {
        with_target(cx, |g, window, cx| g.export_table_to_clipboard(window, cx))
    });
}

/// Opens the Export Data dialog for the active grid's table; `false`
/// without a grid.
pub(crate) fn export_active(cx: &mut App) -> bool {
    let Some((window, grid)) = target(cx) else {
        return false;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            let transfer = grid.read(cx).transfer_target();
            transfer::export::open(transfer, window, cx);
        });
    });
    true
}

/// Opens the Import dialog for the active grid's table; `false` without a
/// grid.
pub(crate) fn import_active(path: Option<String>, cx: &mut App) -> bool {
    let Some((window, grid)) = target(cx) else {
        return false;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            let transfer = grid.read(cx).transfer_target();
            transfer::import::open(transfer, path, window, cx);
        });
    });
    true
}

/// Reads the page again in the grids of `table` of `source` in `window`
/// (after an import).
pub(crate) fn reload_table(
    window: gpui::AnyWindowHandle,
    source: &str,
    table: &TableRef,
    cx: &mut App,
) {
    for grid in super::grids_of(window, cx) {
        grid.update(cx, |g, cx| {
            if g.source == source && &g.table == table {
                g.reload(cx);
            }
        });
    }
}

impl TableGrid {
    /// This grid's table, with the condition and order of the shown page.
    pub(crate) fn transfer_target(&self) -> TransferTarget {
        TransferTarget {
            target: super::GridTarget {
                source: self.source.clone(),
                backend: self.backend,
                handle: self.handle.clone(),
                table: self.table.clone(),
                columns: self.meta.clone(),
            },
            query: self.query.clone(),
        }
    }

    fn write_options(&self, format: Format) -> ExportOptions {
        let dialect = self.dialect();
        let mut options = ExportOptions::new(format, dialect);
        options.table = Some(self.table.qualified(dialect));
        options.key_columns = self
            .meta
            .iter()
            .filter(|c| c.key == super::data::KeyMark::Primary)
            .map(|c| c.name.clone())
            .collect();
        options
    }

    /// The selected cells as columns and rows of values (hidden columns
    /// left out).
    fn selected_values(&self) -> Option<(Vec<Column>, Vec<Row>)> {
        let page = self.page.as_ref()?;
        let selection = self.selection?;
        let shown: Vec<usize> = selection
            .columns()
            .filter(|c| *c < page.columns.len() && !self.is_hidden(*c))
            .collect();
        if shown.is_empty() {
            return None;
        }
        let columns = shown
            .iter()
            .map(|&c| {
                let column = &page.columns[c];
                Column {
                    name: column.name.to_string(),
                    type_name: column.type_name.to_string(),
                    kind: column.kind,
                    nullable: true,
                }
            })
            .collect();
        let rows = selection
            .rows()
            .filter_map(|r| page.rows.get(r))
            .map(|row| {
                shown
                    .iter()
                    .map(|&c| row.get(c).cloned().unwrap_or(Value::Null))
                    .collect()
            })
            .collect();
        Some((columns, rows))
    }

    /// The selection written in `format` (else the toolbar's extractor).
    pub(crate) fn selection_text(&self, format: Option<Format>) -> Option<String> {
        let (columns, rows) = self.selected_values()?;
        let options = self.write_options(format.unwrap_or(self.extractor));
        Some(writers::to_string(&options, &columns, &rows))
    }

    pub(super) fn copy_selection(&mut self, format: Option<Format>, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text(format) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn set_extractor(&mut self, format: Format, cx: &mut Context<Self>) {
        if self.extractor != format {
            self.extractor = format;
            cx.notify();
        }
    }

    fn export_table_to_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transfer = self.transfer_target();
        let request = db_session::ExportRequest {
            source: db_session::ExportSource::Table(transfer.query.clone()),
            options: self.write_options(self.extractor),
            target: ExportTarget::Memory,
            limit: None,
        };
        let main = window.window_handle();
        transfer::run::export(&transfer, request, transfer::run::Goal::Clipboard, main, cx);
    }

    // ---- the menus ---------------------------------------------------------------

    fn extractor_choices(&self) -> Vec<MenuItem> {
        Format::ALL
            .into_iter()
            .map(|f| {
                let mut item = MenuItem::action(
                    f.title(),
                    SetExtractor {
                        format: f.id().to_owned(),
                    },
                );
                if f == self.extractor {
                    item = item.icon(IconName::CHECKED);
                }
                item
            })
            .collect()
    }

    /// The copy rows of the cell menu (shot DB48).
    pub(super) fn copy_menu_items(&self) -> Vec<MenuItem> {
        let title = self.extractor.title();
        let has_selection = self.selection.is_some();
        let copy_label = format!("Copy Using Data Extractor ({title})");
        let mut items = vec![if has_selection {
            MenuItem::action(copy_label, CopyData).icon(IconName::COPY)
        } else {
            MenuItem::unavailable(copy_label)
        }];
        items.push(MenuItem::submenu(
            format!("Change Data Extractor ({title})"),
            self.extractor_choices(),
        ));
        if has_selection {
            items.push(MenuItem::submenu(
                "Copy As",
                Format::ALL
                    .into_iter()
                    .map(|f| {
                        MenuItem::action(
                            f.title(),
                            CopyAs {
                                format: f.id().to_owned(),
                            },
                        )
                    })
                    .collect(),
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            "Export Table to Clipboard",
            ExportTableToClipboard,
        ));
        items
    }

    fn open_menu(
        &mut self,
        items: Vec<MenuItem>,
        position: Point<Pixels>,
        corner: gpui::Corner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |g, _, _: &DismissEvent, cx| {
            g.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position,
            corner,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The extractor list under the toolbar's button.
    pub(super) fn show_extractor_menu(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.extractor_choices();
        let at = at.unwrap_or_else(|| point(px(400.0), px(40.0)));
        self.open_menu(items, at, gpui::Corner::TopRight, window, cx);
    }

    // ---- the toolbar -------------------------------------------------------------

    /// The extractor button: its name and a chevron.
    pub(super) fn extractor_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        div()
            .id("grid-extractor")
            .debug_selector(|| "grid-extractor".to_owned())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.0))
            .h(px(24.0))
            .px(px(4.0))
            .rounded(theme.metric(themes::Metric::ButtonRadius))
            .text_size(theme.ui_size() - px(1.0))
            .text_color(theme.color(UiRole::Text))
            .hover(|s| s.bg(cx.theme().color(UiRole::Hover)))
            .tooltip(Tooltip::text("Data Extractor", None))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|g, e: &MouseDownEvent, window, cx| {
                    g.show_extractor_menu(
                        Some(point(e.position.x + px(16.0), e.position.y + px(14.0))),
                        window,
                        cx,
                    );
                }),
            )
            .child(self.extractor.title())
            .child(Icon::new(IconName::CHEVRON_DOWN).size(px(10.0)))
            .into_any_element()
    }

    /// The Export Data and Import buttons.
    pub(super) fn transfer_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.page.is_some();
        let button = |id: &'static str,
                      icon: IconName,
                      tip: &'static str,
                      action: Box<dyn gpui::Action>,
                      cx: &mut Context<Self>| {
            Self::tool(id, icon, tip, on, cx)
                .when(on, move |d| {
                    d.on_click(cx.listener(move |_, _, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx)
                    }))
                })
                .when(!on, |d| d.opacity(DISABLED_ALPHA))
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .gap(px(2.0))
            .child(button(
                "grid-export",
                icons::EXPORT_DATA,
                "Export Data",
                Box::new(ExportData {
                    source: None,
                    schema: None,
                    table: None,
                }),
                cx,
            ))
            .child(button(
                "grid-import",
                icons::IMPORT_DATA,
                "Import Data from File",
                Box::new(ImportData {
                    source: None,
                    schema: None,
                    table: None,
                    path: None,
                }),
                cx,
            ))
            .into_any_element()
    }
}
