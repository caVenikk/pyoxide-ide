use db_core::Dialect;
use serde::{Deserialize, Serialize};

/// A data extractor: the format a result is written or copied in. The
/// order of [`Format::ALL`] is the order of the extractor list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    SqlInserts,
    SqlUpdates,
    WhereClause,
    Csv,
    Tsv,
    PipeSeparated,
    SemicolonSeparated,
    Html,
    Json,
    Markdown,
}

impl Format {
    pub const ALL: [Format; 10] = [
        Format::SqlInserts,
        Format::SqlUpdates,
        Format::WhereClause,
        Format::Csv,
        Format::Tsv,
        Format::PipeSeparated,
        Format::SemicolonSeparated,
        Format::Html,
        Format::Json,
        Format::Markdown,
    ];

    /// The name the extractor list shows.
    pub fn title(self) -> &'static str {
        match self {
            Format::SqlInserts => "SQL Inserts",
            Format::SqlUpdates => "SQL Updates",
            Format::WhereClause => "Where Clause",
            Format::Csv => "CSV",
            Format::Tsv => "TSV",
            Format::PipeSeparated => "Pipe-separated",
            Format::SemicolonSeparated => "Semicolon-separated",
            Format::Html => "HTML",
            Format::Json => "JSON",
            Format::Markdown => "Markdown",
        }
    }

    /// The name used by commands and in saved settings.
    pub fn id(self) -> &'static str {
        match self {
            Format::SqlInserts => "sql-inserts",
            Format::SqlUpdates => "sql-updates",
            Format::WhereClause => "where-clause",
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::PipeSeparated => "pipe-separated",
            Format::SemicolonSeparated => "semicolon-separated",
            Format::Html => "html",
            Format::Json => "json",
            Format::Markdown => "markdown",
        }
    }

    pub fn from_id(id: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The extension of a file written in this format.
    pub fn extension(self) -> &'static str {
        match self {
            Format::SqlInserts | Format::SqlUpdates => "sql",
            Format::WhereClause | Format::PipeSeparated => "txt",
            Format::Csv | Format::SemicolonSeparated => "csv",
            Format::Tsv => "tsv",
            Format::Html => "html",
            Format::Json => "json",
            Format::Markdown => "md",
        }
    }

    /// The field separator of the delimited formats.
    pub fn delimiter(self) -> Option<char> {
        match self {
            Format::Csv => Some(','),
            Format::Tsv => Some('\t'),
            Format::PipeSeparated => Some('|'),
            Format::SemicolonSeparated => Some(';'),
            _ => None,
        }
    }

    /// Rows as lines of cells (the formats the header, row header and
    /// transpose options apply to).
    pub fn is_tabular(self) -> bool {
        self.delimiter().is_some() || matches!(self, Format::Html | Format::Markdown)
    }

    /// SQL text, written with the dialect's literals.
    pub fn is_sql(self) -> bool {
        matches!(
            self,
            Format::SqlInserts | Format::SqlUpdates | Format::WhereClause
        )
    }
}

/// What a writer is told besides the format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub format: Format,
    /// A first line with the column names (tabular formats; Markdown and
    /// HTML always have one).
    pub column_header: bool,
    /// A first column with the row numbers.
    pub row_header: bool,
    /// Columns become lines (holds every row in memory).
    pub transpose: bool,
    /// How NULL is written in the tabular formats.
    pub null_text: String,
    /// Quote every field of the delimited formats, not only those that
    /// need it.
    pub always_quote: bool,
    /// The table of SQL statements, as it is written in SQL (quoted).
    pub table: Option<String>,
    pub dialect: Dialect,
    /// The columns that identify a row in `UPDATE` and `WHERE` (all
    /// columns when empty).
    pub key_columns: Vec<String>,
}

impl ExportOptions {
    pub fn new(format: Format, dialect: Dialect) -> Self {
        Self {
            format,
            column_header: false,
            row_header: false,
            transpose: false,
            null_text: String::new(),
            always_quote: false,
            table: None,
            dialect,
            key_columns: Vec::new(),
        }
    }
}
