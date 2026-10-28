//! Moving rows in and out of a table (ADR 0021): the Export Data dialog
//! (the rows of a table, with the grid's condition and order, written in one
//! of the data extractors to a file or the clipboard, with a preview of the
//! first rows) and the Import dialog (a CSV file into an existing table, its
//! columns mapped to the table's). Both run their jobs on the database
//! runtime, never on the UI thread: a job shows in the status bar's
//! background tasks with a cancel button and ends with a balloon.
//!
//! The commands `db.export.open` and `db.import.open` start them from the
//! grid's toolbar, the Database tool window (the selected table) or a
//! control channel (`source`, `schema` and `table` name the table).

pub(crate) mod export;
pub(crate) mod import;
pub(crate) mod run;

use db_core::Dialect;
use db_session::TableQuery;
use gpui::{App, Global};
use serde_json::{Value as Json, json};

use crate::grid::GridTarget;
use crate::grid::data::KeyMark;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Export the rows of a table.
    ExportData { source: Option<String>, schema: Option<String>, table: Option<String> } => "db.export.open", "Export Data\u{2026}",
        "Opens the Export Data dialog for the active table grid (its WHERE and ORDER BY apply), or for table `table` of `schema` in data source `source` (every row), or for the table selected in the Database tool window.",
        r#"{"source": "local-pg", "schema": "demo", "table": "customers"}"#;
    /// Choose the extractor.
    ExportSetFormat { format: String } => "db.export.set-format", "Set Extractor",
        "Chooses the data extractor of the Export Data dialog (`csv`, `tsv`, `pipe-separated`, `semicolon-separated`, `html`, `json`, `markdown`, `sql-inserts`, `sql-updates`, `where-clause`).",
        r#"{"format": "tsv"}"#;
    /// Set an option.
    ExportSetOption { option: String, value: bool } => "db.export.set-option", "Set Export Option",
        "Turns `transpose`, `column-header` or `row-header` of the Export Data dialog on or off.",
        r#"{"option": "column-header", "value": true}"#;
    /// Set the text of NULL.
    ExportSetNullText { text: String } => "db.export.set-null-text", "Set NULL Text",
        "Sets the text the Export Data dialog writes for NULL in the tabular extractors (empty: nothing).",
        r#"{"text": "\\N"}"#;
    /// Set the output file.
    ExportSetPath { path: String } => "db.export.set-path", "Set Output File",
        "Types `path` into the output file field of the Export Data dialog.",
        r#"{"path": "/tmp/customers.csv"}"#;
    /// Choose the output folder.
    ExportBrowse => "db.export.browse", "Browse\u{2026}",
        "Opens the folder chooser of the Export Data dialog.";
    /// Show the extractor list.
    ExportFormatMenu => "db.export.format-menu", "Extractor List",
        "Opens the extractor list of the Export Data dialog.";
    /// Write the file.
    ExportToFile => "db.export.to-file", "Export to File",
        "Closes the Export Data dialog and writes the rows to the output file in the background.";
    /// Copy the rows.
    ExportToClipboard => "db.export.to-clipboard", "Copy to Clipboard",
        "Closes the Export Data dialog and copies the rows to the clipboard in the background.";
    /// Close the dialog.
    ExportCancel => "db.export.cancel", "Cancel", "Closes the Export Data dialog without exporting.";
    /// Import a file.
    ImportData { source: Option<String>, schema: Option<String>, table: Option<String>, path: Option<String> } => "db.import.open", "Import Data from File\u{2026}",
        "Opens the Import dialog for the table of the active table grid, or table `table` of `schema` in data source `source`, or the table selected in the Database tool window; `path` is the file to import (else the file chooser opens).",
        r#"{"source": "local-pg", "schema": "demo", "table": "import_target", "path": "/tmp/data.csv"}"#;
    /// Choose the file.
    ImportSetFile { path: String } => "db.import.set-file", "Set Import File",
        "Chooses the file of the Import dialog.",
        r#"{"path": "/tmp/data.csv"}"#;
    /// Choose the file with the chooser.
    ImportBrowse => "db.import.browse", "Choose File\u{2026}", "Opens the file chooser of the Import dialog.";
    /// Set an option.
    ImportSetOption { option: String, value: bool } => "db.import.set-option", "Set Import Option",
        "Turns `header` (the first row holds column names), `empty-as-null` or `inconvertible-as-null` of the Import dialog on or off.",
        r#"{"option": "header", "value": false}"#;
    /// Set the delimiter.
    ImportSetDelimiter { delimiter: String } => "db.import.set-delimiter", "Set Delimiter",
        "Sets the field delimiter of the Import dialog: `,`, `;`, `|` or `tab`.",
        r#"{"delimiter": ";"}"#;
    /// Set the text of NULL.
    ImportSetNullText { text: String } => "db.import.set-null-text", "Set NULL Text",
        "Sets the text of a field (not quoted) that the Import dialog reads as NULL, besides an empty field (empty: none).",
        r#"{"text": "\\N"}"#;
    /// Pair columns.
    ImportMap { column: String, source: Option<usize> } => "db.import.map", "Map Column",
        "Takes the values of table column `column` in the Import dialog from the file's column `source` (zero-based), or leaves it unfilled.",
        r#"{"column": "name", "source": 1}"#;
    /// Show a column's choices.
    ImportMapMenu { column: String } => "db.import.map-menu", "Column Choices",
        "Opens the list of file columns for table column `column` in the Import dialog.",
        r#"{"column": "name"}"#;
    /// Run the import.
    ImportOk => "db.import.ok", "OK",
        "Closes the Import dialog and imports the file in the background, in one transaction.";
    /// Close the dialog.
    ImportCancel => "db.import.cancel", "Cancel", "Closes the Import dialog without importing.";
    /// The state of the open dialog.
    TransferState => "db.transfer.state", "Export and Import State",
        "Returns the state of the open Export Data or Import dialog, `null` when none is open.";
}

/// What a dialog transfers: a table with the grid's condition and order.
#[derive(Clone)]
pub struct TransferTarget {
    pub target: GridTarget,
    pub query: TableQuery,
}

impl TransferTarget {
    pub fn new(target: GridTarget) -> Self {
        let query = TableQuery::new(target.table.clone());
        Self { target, query }
    }

    pub(crate) fn dialect(&self) -> Dialect {
        self.target.backend.dialect()
    }

    /// `schema.table`, as the dialogs show it.
    pub(crate) fn title(&self) -> String {
        match &self.target.table.schema {
            Some(schema) => format!("{schema}.{}", self.target.table.name),
            None => self.target.table.name.clone(),
        }
    }

    /// The columns of the primary key (they identify a row in `UPDATE`
    /// and `WHERE` statements).
    pub(crate) fn key_columns(&self) -> Vec<String> {
        self.target
            .columns
            .iter()
            .filter(|c| c.key == KeyMark::Primary)
            .map(|c| c.name.clone())
            .collect()
    }
}

/// The dialog that is open, for [`TransferState`].
#[derive(Default)]
struct Open {
    export: Option<gpui::WeakEntity<export::ExportDialog>>,
    import: Option<gpui::WeakEntity<import::ImportDialog>>,
}

impl Global for Open {}

pub(crate) fn set_open_export(
    dialog: Option<gpui::WeakEntity<export::ExportDialog>>,
    cx: &mut App,
) {
    cx.default_global::<Open>().export = dialog;
}

pub(crate) fn set_open_import(
    dialog: Option<gpui::WeakEntity<import::ImportDialog>>,
    cx: &mut App,
) {
    cx.default_global::<Open>().import = dialog;
}

/// The state of the open dialog.
pub(crate) fn state_json(cx: &App) -> Json {
    let Some(open) = cx.try_global::<Open>() else {
        return Json::Null;
    };
    if let Some(d) = open.export.as_ref().and_then(|d| d.upgrade()) {
        return json!({ "dialog": "export", "state": d.read(cx).state_json(cx) });
    }
    if let Some(d) = open.import.as_ref().and_then(|d| d.upgrade()) {
        return json!({ "dialog": "import", "state": d.read(cx).state_json(cx) });
    }
    Json::Null
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register_query(
        cx,
        "db.transfer.state",
        std::rc::Rc::new(|_, _, cx| Ok(state_json(cx))),
    );
    cx.on_action(|a: &ExportData, cx| {
        if a.table.is_none() && crate::grid::export_active(cx) {
            return;
        }
        crate::explorer::with_table(
            a.source.clone(),
            a.schema.clone(),
            a.table.clone(),
            |target, window, cx| {
                let target = TransferTarget::new(target);
                let _ = window.update(cx, |_, window, cx| export::open(target, window, cx));
            },
            cx,
        );
    });
    cx.on_action(|a: &ImportData, cx| {
        let path = a.path.clone();
        if a.table.is_none() && crate::grid::import_active(path.clone(), cx) {
            return;
        }
        crate::explorer::with_table(
            a.source.clone(),
            a.schema.clone(),
            a.table.clone(),
            move |target, window, cx| {
                let target = TransferTarget::new(target);
                let _ = window.update(cx, |_, window, cx| import::open(target, path, window, cx));
            },
            cx,
        );
    });
}
