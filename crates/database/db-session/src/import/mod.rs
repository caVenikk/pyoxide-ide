//! Import of a delimited text file into an existing table (ADR 0021): a
//! streaming reader with a guessed delimiter and encoding, a preview of the
//! first rows, a mapping of file columns to table columns, and a job that
//! inserts in batches with bound parameters in one transaction on a
//! connection of its own. A failure rolls everything back and names the row
//! and the reason; a cancel does the same.

mod convert;
mod csv;
mod encoding;
mod job;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::Serialize;

pub use convert::convert;
pub use csv::{CsvFormat, CsvReader, Field, Record, guess_delimiter};
pub use encoding::{Encoding, detect as detect_encoding};
pub use job::{ImportDone, ImportEvent, ImportFailure, ImportOutcome, ImportRequest, ImportSink};

/// How the file is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFormat {
    pub csv: CsvFormat,
    /// The first record holds the column names.
    pub header: bool,
    /// A field (not quoted) with this text is NULL.
    pub null_text: Option<String>,
    /// A field that is empty and not quoted is NULL (a quoted empty field is
    /// an empty text, as the export writes it).
    pub empty_as_null: bool,
}

impl Default for ImportFormat {
    fn default() -> Self {
        Self {
            csv: CsvFormat::default(),
            header: true,
            null_text: None,
            empty_as_null: true,
        }
    }
}

impl ImportFormat {
    /// The value of a field: `None` for NULL.
    pub fn value_of(&self, field: &Field) -> Option<String> {
        if !field.quoted {
            if field.text.is_empty() && self.empty_as_null {
                return None;
            }
            if self.null_text.as_deref() == Some(field.text.as_str()) {
                return None;
            }
        }
        Some(field.text.clone())
    }
}

/// What a file looks like with the options chosen: its encoding, the column
/// names and the first rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preview {
    pub encoding: Encoding,
    /// The names of the first record when it is a header; `Column1`, ...
    /// otherwise.
    pub header: Vec<String>,
    /// Data rows; `None` is NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows follow the preview.
    pub more: bool,
}

/// The delimiter and the encoding a file most likely has: its start is
/// looked at, never the whole file. A `.tsv` file is tab-separated.
pub fn detect_format(path: &Path) -> io::Result<(ImportFormat, Encoding)> {
    let mut sample = vec![0u8; 64 * 1024];
    let mut n = 0;
    let mut file = File::open(path)?;
    while n < sample.len() {
        let got = file.read(&mut sample[n..])?;
        if got == 0 {
            break;
        }
        n += got;
    }
    sample.truncate(n);
    let (encoding, _) = encoding::detect(&sample);
    let (mut reader, _) = encoding::open(io::Cursor::new(sample), Some(encoding))?;
    let mut text = Vec::new();
    reader.read_to_end(&mut text)?;
    let mut format = ImportFormat::default();
    format.csv.delimiter = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tsv"))
    {
        b'\t'
    } else {
        guess_delimiter(&text)
    };
    Ok((format, encoding))
}

/// Opens `path` for reading records.
pub(crate) fn open_records(
    path: &Path,
    format: &ImportFormat,
    encoding: Option<Encoding>,
) -> io::Result<(CsvReader<Box<dyn io::BufRead + Send>>, Encoding)> {
    let file = File::open(path)?;
    let (reader, encoding) = encoding::open(file, encoding)?;
    Ok((CsvReader::new(reader, format.csv), encoding))
}

/// The first `max_rows` data rows of `path` with the options given.
pub fn preview(
    path: &Path,
    format: &ImportFormat,
    encoding: Option<Encoding>,
    max_rows: usize,
) -> io::Result<Preview> {
    let (mut reader, encoding) = open_records(path, format, encoding)?;
    let mut header: Vec<String> = Vec::new();
    if format.header
        && let Some(r) = reader.next_record()?
    {
        header = r.fields.into_iter().map(|f| f.text).collect();
    }
    let mut rows = Vec::new();
    let mut more = false;
    while let Some(r) = reader.next_record()? {
        if rows.len() == max_rows {
            more = true;
            break;
        }
        rows.push(
            r.fields
                .iter()
                .map(|f| format.value_of(f))
                .collect::<Vec<_>>(),
        );
    }
    let width = rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0);
    if !format.header {
        header = (1..=width).map(|i| format!("Column{i}")).collect();
    } else {
        while header.len() < width {
            header.push(format!("Column{}", header.len() + 1));
        }
    }
    Ok(Preview {
        encoding,
        header,
        rows,
        more,
    })
}

/// Which file column fills a table column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMap {
    /// The table's column.
    pub column: String,
    /// The file's column (zero-based); `None`: not filled, the table's
    /// default applies.
    pub source: Option<usize>,
}

/// Pairs the table's columns with the file's: by name (case does not
/// matter) when the file has a header, by position otherwise. Columns
/// without a partner are left out.
pub fn auto_map(columns: &[String], file_columns: &[String], has_header: bool) -> Vec<ColumnMap> {
    columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let source = if has_header {
                file_columns
                    .iter()
                    .position(|f| f.trim().eq_ignore_ascii_case(column))
            } else {
                (i < file_columns.len()).then_some(i)
            };
            ColumnMap {
                column: column.clone(),
                source,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
