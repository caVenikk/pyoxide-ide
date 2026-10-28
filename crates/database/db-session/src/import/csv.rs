//! A streaming reader of delimited text: one record at a time from a
//! `BufRead`, quoted fields with doubled quotes and line breaks inside,
//! lines ended by `\n`, `\r\n` or `\r`. The text is UTF-8 (see
//! [`super::encoding`]); bytes that are not are replaced.

use std::io::{self, BufRead};

/// How the text is split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvFormat {
    /// An ASCII character.
    pub delimiter: u8,
    /// The quote character; `None`: fields are never quoted.
    pub quote: Option<u8>,
}

impl Default for CsvFormat {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: Some(b'"'),
        }
    }
}

/// One field: its text and whether it was written in quotes (a quoted
/// empty field is an empty text, a bare one can mean NULL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    pub quoted: bool,
}

/// One record and where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub fields: Vec<Field>,
    /// The line of the file the record starts on, from 1.
    pub line: u64,
}

pub struct CsvReader<R> {
    inner: R,
    format: CsvFormat,
    /// The line the next record starts on.
    line: u64,
    /// Bytes consumed.
    bytes: u64,
}

impl<R: BufRead> CsvReader<R> {
    pub fn new(inner: R, format: CsvFormat) -> Self {
        Self {
            inner,
            format,
            line: 1,
            bytes: 0,
        }
    }

    /// Bytes of the text read so far.
    pub fn bytes_read(&self) -> u64 {
        self.bytes
    }

    fn peek(&mut self) -> io::Result<Option<u8>> {
        Ok(self.inner.fill_buf()?.first().copied())
    }

    fn bump(&mut self) {
        self.inner.consume(1);
        self.bytes += 1;
    }

    fn syntax(&self, line: u64, message: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("line {line}: {message}"),
        )
    }

    /// The next record; blank lines are skipped. `None` at the end.
    pub fn next_record(&mut self) -> io::Result<Option<Record>> {
        loop {
            let start = self.line;
            let Some(fields) = self.read_fields(start)? else {
                return Ok(None);
            };
            if fields.len() == 1 && fields[0].text.is_empty() && !fields[0].quoted {
                continue;
            }
            return Ok(Some(Record {
                fields,
                line: start,
            }));
        }
    }

    fn read_fields(&mut self, start: u64) -> io::Result<Option<Vec<Field>>> {
        if self.peek()?.is_none() {
            return Ok(None);
        }
        let mut fields = Vec::new();
        loop {
            let (field, end) = self.read_field(start)?;
            fields.push(field);
            match end {
                End::Delimiter => {}
                End::Line | End::Eof => return Ok(Some(fields)),
            }
        }
    }

    fn read_field(&mut self, start: u64) -> io::Result<(Field, End)> {
        let mut raw: Vec<u8> = Vec::new();
        let mut quoted = false;
        if self.format.quote.is_some() && self.peek()? == self.format.quote {
            quoted = true;
            let quote = self.format.quote.unwrap_or(b'"');
            self.bump();
            loop {
                let Some(b) = self.peek()? else {
                    return Err(self.syntax(start, "a quoted field is not closed"));
                };
                self.bump();
                if b == quote {
                    if self.peek()? == Some(quote) {
                        raw.push(quote);
                        self.bump();
                    } else {
                        break;
                    }
                } else {
                    if b == b'\n' {
                        self.line += 1;
                    }
                    raw.push(b);
                }
            }
        }
        // The rest of the field up to the delimiter or the line's end (after
        // a closing quote this is lenient: text that follows is kept).
        let end = loop {
            match self.peek()? {
                None => break End::Eof,
                Some(b) if b == self.format.delimiter => {
                    self.bump();
                    break End::Delimiter;
                }
                Some(b'\n') => {
                    self.bump();
                    self.line += 1;
                    break End::Line;
                }
                Some(b'\r') => {
                    self.bump();
                    if self.peek()? == Some(b'\n') {
                        self.bump();
                    }
                    self.line += 1;
                    break End::Line;
                }
                Some(b) => {
                    raw.push(b);
                    self.bump();
                }
            }
        };
        let text = match String::from_utf8(raw) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        Ok((Field { text, quoted }, end))
    }
}

enum End {
    Delimiter,
    Line,
    Eof,
}

/// The delimiter that splits the first records of `sample` the most
/// consistently: every record the same number of fields, more than one.
/// A comma when none does.
pub fn guess_delimiter(sample: &[u8]) -> u8 {
    let mut best = (0usize, 0usize, b',');
    for delimiter in [b',', b';', b'\t', b'|'] {
        let mut reader = CsvReader::new(
            io::Cursor::new(sample),
            CsvFormat {
                delimiter,
                ..CsvFormat::default()
            },
        );
        let mut counts = Vec::new();
        while counts.len() < 20 {
            match reader.next_record() {
                Ok(Some(r)) => counts.push(r.fields.len()),
                // A cut last record of the sample is not evidence.
                _ => break,
            }
        }
        let Some(&first) = counts.first() else {
            continue;
        };
        if first < 2 {
            continue;
        }
        let same = counts.iter().filter(|c| **c == first).count();
        // More consistent records first, then more fields.
        let score = (same, first, delimiter);
        if (score.0, score.1) > (best.0, best.1) {
            best = score;
        }
    }
    best.2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(text: &str, format: CsvFormat) -> Vec<Vec<(String, bool)>> {
        let mut r = CsvReader::new(io::Cursor::new(text.as_bytes()), format);
        let mut out = Vec::new();
        while let Some(rec) = r.next_record().unwrap() {
            out.push(rec.fields.into_iter().map(|f| (f.text, f.quoted)).collect());
        }
        out
    }

    fn f(text: &str) -> (String, bool) {
        (text.to_owned(), false)
    }

    fn q(text: &str) -> (String, bool) {
        (text.to_owned(), true)
    }

    #[test]
    fn quotes_commas_and_line_breaks() {
        let text = "id,name,note\n1,\"Ada, L.\",\n2,\"Line\nbreak\",\"say \"\"hi\"\" now\"\n3,Grace,\"\"\n";
        assert_eq!(
            records(text, CsvFormat::default()),
            vec![
                vec![f("id"), f("name"), f("note")],
                vec![f("1"), q("Ada, L."), f("")],
                vec![f("2"), q("Line\nbreak"), q("say \"hi\" now")],
                vec![f("3"), f("Grace"), q("")],
            ]
        );
    }

    #[test]
    fn line_endings_and_blank_lines() {
        assert_eq!(
            records("a,b\r\n\r\nc,d\re,f", CsvFormat::default()),
            vec![
                vec![f("a"), f("b")],
                vec![f("c"), f("d")],
                vec![f("e"), f("f")]
            ]
        );
    }

    #[test]
    fn lines_are_counted_through_quoted_breaks() {
        let mut r = CsvReader::new(
            io::Cursor::new("a\n\"x\ny\"\nb\n".as_bytes()),
            CsvFormat::default(),
        );
        let lines: Vec<u64> = std::iter::from_fn(|| r.next_record().unwrap())
            .map(|r| r.line)
            .collect();
        assert_eq!(lines, [1, 2, 4]);
    }

    #[test]
    fn an_open_quote_is_an_error_naming_the_line() {
        let mut r = CsvReader::new(
            io::Cursor::new("a\n\"oops".as_bytes()),
            CsvFormat::default(),
        );
        r.next_record().unwrap();
        let e = r.next_record().unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
    }

    #[test]
    fn unicode_and_a_missing_final_newline() {
        assert_eq!(
            records("é,日本\nГ,😀", CsvFormat::default()),
            vec![vec![f("é"), f("日本")], vec![f("Г"), f("😀")]]
        );
    }

    #[test]
    fn no_quote_character() {
        let format = CsvFormat {
            quote: None,
            ..CsvFormat::default()
        };
        assert_eq!(records("a,\"b\n", format), vec![vec![f("a"), f("\"b")]]);
    }

    #[test]
    fn the_delimiter_is_guessed() {
        assert_eq!(guess_delimiter(b"a;b;c\n1;2;3\n4;5;6\n"), b';');
        assert_eq!(guess_delimiter(b"a\tb\n1\t2\n"), b'\t');
        assert_eq!(guess_delimiter(b"a|b|c\n1|2|3\n"), b'|');
        assert_eq!(guess_delimiter(b"a,b,c\n1,2,3\n"), b',');
        // Commas inside quotes and decimal commas do not decide.
        assert_eq!(
            guess_delimiter(b"name;price\n\"a, b\";1,5\n\"c, d\";2,5\n"),
            b';'
        );
        assert_eq!(guess_delimiter(b"just text\n"), b',');
    }
}
