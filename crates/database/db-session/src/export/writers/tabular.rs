//! CSV, TSV, pipe- and semicolon-separated text, Markdown and HTML: rows as
//! lines of cells, with the header, row header and transpose options.

use std::io::{self, Write};

use db_core::{Column, Value};

use super::RowWriter;
use super::literal::value_text;
use crate::export::format::{ExportOptions, Format};

/// How one line of cells is written.
trait CellFormat: Send {
    fn begin(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
    /// The first line: the column names (or blank cells).
    fn header(&mut self, out: &mut dyn Write, cells: &[String]) -> io::Result<()>;
    /// A line; `None` is NULL.
    fn line(&mut self, out: &mut dyn Write, cells: &[Option<String>]) -> io::Result<()>;
    fn end(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}

pub(super) struct TextWriter {
    format: Box<dyn CellFormat>,
    names: Vec<String>,
    /// Always a header line (Markdown, HTML).
    header_always: bool,
    column_header: bool,
    row_header: bool,
    transpose: bool,
    count: u64,
    /// The cells of every row, when transposing.
    held: Vec<Vec<Option<String>>>,
}

impl TextWriter {
    pub(super) fn new(options: &ExportOptions, columns: &[Column]) -> Self {
        let aligns: Vec<bool> = if options.transpose {
            vec![false; columns.len()]
        } else {
            columns.iter().map(|c| c.kind.is_numeric()).collect()
        };
        let format: Box<dyn CellFormat> = match options.format {
            Format::Markdown => Box::new(Markdown {
                null_text: options.null_text.clone(),
                right: aligns,
                row_header: options.row_header,
            }),
            Format::Html => Box::new(Html {
                null_text: options.null_text.clone(),
            }),
            f => Box::new(Delimited {
                delimiter: f.delimiter().unwrap_or(','),
                null_text: options.null_text.clone(),
                always_quote: options.always_quote,
            }),
        };
        Self {
            format,
            names: columns.iter().map(|c| c.name.clone()).collect(),
            header_always: matches!(options.format, Format::Markdown | Format::Html),
            column_header: options.column_header,
            row_header: options.row_header,
            transpose: options.transpose,
            count: 0,
            held: Vec::new(),
        }
    }

    fn cells(&self, row: &[Value]) -> Vec<Option<String>> {
        row.iter()
            .map(|v| (!v.is_null()).then(|| value_text(v).into_owned()))
            .collect()
    }
}

impl RowWriter for TextWriter {
    fn begin(&mut self, out: &mut dyn Write) -> io::Result<()> {
        self.format.begin(out)?;
        if !self.transpose && (self.column_header || self.header_always) {
            let mut cells = Vec::with_capacity(self.names.len() + 1);
            if self.row_header {
                cells.push(String::new());
            }
            cells.extend(self.names.iter().cloned());
            self.format.header(out, &cells)?;
        }
        Ok(())
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()> {
        self.count += 1;
        let mut cells = self.cells(row);
        if self.transpose {
            self.held.push(cells);
            return Ok(());
        }
        if self.row_header {
            cells.insert(0, Some(self.count.to_string()));
        }
        self.format.line(out, &cells)
    }

    fn finish(&mut self, out: &mut dyn Write) -> io::Result<()> {
        if self.transpose {
            let held = std::mem::take(&mut self.held);
            let blank = usize::from(self.column_header);
            if self.row_header || self.header_always {
                let mut cells = vec![String::new(); blank];
                if self.row_header {
                    cells.extend((1..=held.len()).map(|n| n.to_string()));
                } else {
                    cells.extend(std::iter::repeat_n(String::new(), held.len()));
                }
                self.format.header(out, &cells)?;
            }
            for (i, name) in self.names.iter().enumerate() {
                let mut cells: Vec<Option<String>> = Vec::with_capacity(held.len() + 1);
                if self.column_header {
                    cells.push(Some(name.clone()));
                }
                cells.extend(held.iter().map(|r| r.get(i).cloned().flatten()));
                self.format.line(out, &cells)?;
            }
        }
        self.format.end(out)
    }
}

// ---- delimited --------------------------------------------------------------

struct Delimited {
    delimiter: char,
    null_text: String,
    always_quote: bool,
}

impl Delimited {
    /// A field: quoted when it holds the delimiter, a quote or a line break,
    /// when it is empty, and when it is the NULL text, so that an export
    /// read back is equal (a bare field is NULL, a quoted one is a text).
    fn field(&self, text: Option<&str>) -> String {
        let (value, is_null) = match text {
            Some(t) => (t, false),
            None => (self.null_text.as_str(), true),
        };
        // An empty text is quoted whatever the NULL text is.
        let looks_null = !is_null && (value == self.null_text || value.is_empty());
        let needs = self.always_quote && !is_null
            || value.contains(self.delimiter)
            || value.contains(['"', '\n', '\r'])
            || looks_null;
        if needs {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_owned()
        }
    }

    fn write_line(&self, out: &mut dyn Write, fields: Vec<String>) -> io::Result<()> {
        let mut delimiter = [0u8; 4];
        let delimiter = self.delimiter.encode_utf8(&mut delimiter);
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                out.write_all(delimiter.as_bytes())?;
            }
            out.write_all(f.as_bytes())?;
        }
        out.write_all(b"\n")
    }
}

impl CellFormat for Delimited {
    fn header(&mut self, out: &mut dyn Write, cells: &[String]) -> io::Result<()> {
        // Names are text, never NULL; an empty name reads back as an empty
        // text.
        let fields = cells
            .iter()
            .map(|c| {
                if c.is_empty() {
                    String::new()
                } else {
                    self.field(Some(c))
                }
            })
            .collect();
        self.write_line(out, fields)
    }

    fn line(&mut self, out: &mut dyn Write, cells: &[Option<String>]) -> io::Result<()> {
        let fields = cells.iter().map(|c| self.field(c.as_deref())).collect();
        self.write_line(out, fields)
    }
}

// ---- Markdown ---------------------------------------------------------------

struct Markdown {
    null_text: String,
    /// Right-aligned columns (numbers).
    right: Vec<bool>,
    row_header: bool,
}

impl Markdown {
    fn escape(text: &str) -> String {
        text.replace('|', "\\|")
            .replace("\r\n", "<br>")
            .replace(['\n', '\r'], "<br>")
    }

    fn write_line(out: &mut dyn Write, cells: Vec<String>) -> io::Result<()> {
        write!(out, "|")?;
        for c in cells {
            write!(out, " {c} |")?;
        }
        out.write_all(b"\n")
    }
}

impl CellFormat for Markdown {
    fn header(&mut self, out: &mut dyn Write, cells: &[String]) -> io::Result<()> {
        Self::write_line(out, cells.iter().map(|c| Self::escape(c)).collect())?;
        let offset = usize::from(self.row_header);
        let rule = (0..cells.len())
            .map(|i| {
                if i >= offset && self.right.get(i - offset).copied().unwrap_or(false) {
                    "---:".to_owned()
                } else {
                    "---".to_owned()
                }
            })
            .collect();
        Self::write_line(out, rule)
    }

    fn line(&mut self, out: &mut dyn Write, cells: &[Option<String>]) -> io::Result<()> {
        Self::write_line(
            out,
            cells
                .iter()
                .map(|c| Self::escape(c.as_deref().unwrap_or(&self.null_text)))
                .collect(),
        )
    }
}

// ---- HTML -------------------------------------------------------------------

struct Html {
    null_text: String,
}

fn html_escape(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\n' => s.push_str("<br>\n"),
            c => s.push(c),
        }
    }
    s
}

impl CellFormat for Html {
    fn begin(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(
            b"<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"UTF-8\">\n<style>\n\
              table { border-collapse: collapse; }\n\
              th, td { border: 1px solid #999; padding: 2px 6px; text-align: left; vertical-align: top; }\n\
              th { background: #eee; }\n\
              </style>\n</head>\n<body>\n<table>\n",
        )
    }

    fn header(&mut self, out: &mut dyn Write, cells: &[String]) -> io::Result<()> {
        out.write_all(b"<tr>")?;
        for c in cells {
            write!(out, "<th>{}</th>", html_escape(c))?;
        }
        out.write_all(b"</tr>\n")
    }

    fn line(&mut self, out: &mut dyn Write, cells: &[Option<String>]) -> io::Result<()> {
        out.write_all(b"<tr>")?;
        for c in cells {
            write!(
                out,
                "<td>{}</td>",
                html_escape(c.as_deref().unwrap_or(&self.null_text))
            )?;
        }
        out.write_all(b"</tr>\n")
    }

    fn end(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"</table>\n</body>\n</html>\n")
    }
}
