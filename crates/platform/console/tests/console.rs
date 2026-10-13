//! The console text model through its public API.

use std::sync::Arc;
use std::time::{Duration, Instant};

use console::{
    Changes, Color, Console, ContentKind, Link, LinkFilter, LinkTarget, Style, UrlFilter,
};

use ContentKind::{Error, Normal, System, UserInput};

fn lines(c: &Console) -> Vec<String> {
    c.lines().map(|l| l.text().to_owned()).collect()
}

fn feed(chunks: &[(ContentKind, &str)]) -> Console {
    let mut c = Console::new();
    for (kind, text) in chunks {
        c.push(*kind, text.as_bytes());
    }
    c
}

#[test]
fn starts_with_one_empty_line() {
    let c = Console::new();
    assert_eq!(c.line_count(), 1);
    assert!(c.is_empty());
    assert_eq!(c.len(), 0);
    assert_eq!(c.text(), "");
}

#[test]
fn output_of_both_streams_shares_lines() {
    let c = feed(&[(Normal, "a\nb"), (Error, "c\n"), (Normal, "d")]);
    assert_eq!(lines(&c), ["a", "bc", "d"]);
    let runs: Vec<_> = c
        .line(1)
        .unwrap()
        .runs()
        .iter()
        .map(|r| (r.range.clone(), r.kind))
        .collect();
    assert_eq!(runs, [(0..1, Normal), (1..2, Error)]);
    assert_eq!(c.text(), "a\nbc\nd");
    assert_eq!(c.len(), 6);
}

#[test]
fn system_lines_and_user_input_are_not_decoded() {
    let mut c = Console::new();
    c.push_str(System, "/p/.venv/bin/python /p/main.py \n");
    c.push(Normal, b"Name: ");
    c.push_str(UserInput, "World\n");
    c.push_str(System, "\x1b[31mx");
    assert_eq!(
        lines(&c),
        [
            "/p/.venv/bin/python /p/main.py ",
            "Name: World",
            "\x1b[31mx"
        ]
    );
    let kinds: Vec<_> = c.line(1).unwrap().runs().iter().map(|r| r.kind).collect();
    assert_eq!(kinds, [Normal, UserInput]);
    assert!(c.line(2).unwrap().runs()[0].style == Style::default());
}

#[test]
fn carriage_return_clears_the_line() {
    // Progress bars: the line is replaced, not overwritten by column.
    let c = feed(&[(Normal, "Progress 10%\rProgress 5")]);
    assert_eq!(lines(&c), ["Progress 5"]);
    let c = feed(&[(Normal, "a\nlong text\rxy\n")]);
    assert_eq!(lines(&c), ["a", "xy", ""]);
    // \r\n is a line break, also across chunks.
    let c = feed(&[(Normal, "one\r\ntwo\r"), (Normal, "\nthree")]);
    assert_eq!(lines(&c), ["one", "two", "three"]);
    // A held \r followed by other text clears the line.
    let c = feed(&[(Normal, "10%\r"), (Normal, "20%")]);
    assert_eq!(lines(&c), ["20%"]);
    // ... and is applied when the output ends.
    let mut c = feed(&[(Normal, "done\r")]);
    assert_eq!(lines(&c), ["done"]);
    c.finish();
    assert_eq!(lines(&c), [""]);
}

#[test]
fn backspace_deletes_the_previous_character_of_the_line() {
    let c = feed(&[(Normal, "abc\u{8}\u{8}d")]);
    assert_eq!(lines(&c), ["ad"]);
    let c = feed(&[(Normal, "a\n\u{8}b")]);
    assert_eq!(lines(&c), ["a", "b"]);
    let c = feed(&[(Normal, "xé"), (Normal, "\u{8}\u{8}\u{8}y")]);
    assert_eq!(lines(&c), ["y"]);
    assert_eq!(c.len(), 1);
}

#[test]
fn ansi_state_is_per_stream() {
    let c = feed(&[
        (Normal, "\x1b[31m"),
        (Error, "e"),
        (Normal, "o\x1b[0m"),
        (Error, "\x1b[1mE"),
    ]);
    let runs: Vec<_> = c
        .line(0)
        .unwrap()
        .runs()
        .iter()
        .map(|r| (r.kind, r.style))
        .collect();
    let red = Style {
        fg: Some(Color::Ansi(1)),
        ansi: true,
        ..Style::default()
    };
    let bold = Style {
        bold: true,
        ansi: true,
        ..Style::default()
    };
    assert_eq!(
        runs,
        [(Error, Style::default()), (Normal, red), (Error, bold)]
    );
    assert_eq!(lines(&c), ["eoE"]);
}

#[test]
fn utf8_and_escapes_split_between_chunks() {
    let text = "π \x1b[32mgreen\x1b[0m ✓\n";
    let bytes = text.as_bytes();
    for split in 1..bytes.len() {
        let mut c = Console::new();
        c.push(Normal, &bytes[..split]);
        c.push(Normal, &bytes[split..]);
        assert_eq!(lines(&c), ["π green ✓", ""], "split at {split}");
    }
}

#[test]
fn finish_completes_the_streams() {
    let mut c = Console::new();
    c.push(Normal, b"a\xe2\x82");
    c.push(Error, b"b\x1b[3");
    c.finish();
    assert_eq!(lines(&c), ["ab\u{FFFD}"]);
}

#[test]
fn changes_tell_what_to_reload() {
    let mut c = Console::new();
    assert_eq!(c.take_changes(), None);
    c.push(Normal, b"a\nb");
    assert_eq!(
        c.take_changes(),
        Some(Changes {
            changed_from: 0,
            ..Changes::default()
        })
    );
    assert_eq!(c.take_changes(), None);
    c.push(Normal, b"c\nd\n");
    assert_eq!(c.take_changes().unwrap().changed_from, 1);
    c.clear();
    c.push(Normal, b"x");
    let changes = c.take_changes().unwrap();
    assert!(changes.cleared);
    assert_eq!(lines(&c), ["x"]);
}

#[test]
fn cycle_buffer_cuts_the_oldest_text_exactly() {
    let mut c = Console::with_cycle_buffer(Some(10));
    c.push(Normal, b"0123456789\nabc");
    assert_eq!(lines(&c), ["456789", "abc"]);
    assert_eq!(c.len(), 10);
    let changes = c.take_changes().unwrap();
    assert!(changes.first_line_cut);
    assert_eq!(changes.trimmed, 0);
    c.push(Normal, b"\nxyz\n12");
    assert_eq!(lines(&c), ["abc", "xyz", "12"]);
    assert_eq!(c.len(), 10);
    let changes = c.take_changes().unwrap();
    assert_eq!(changes.trimmed, 1);
    assert!(!changes.first_line_cut);
    assert_eq!(changes.changed_from, 0);
    c.push(Normal, b"3");
    assert_eq!(lines(&c), ["bc", "xyz", "123"]);
    let changes = c.take_changes().unwrap();
    assert_eq!((changes.trimmed, changes.first_line_cut), (0, true));
    assert_eq!(changes.changed_from, 2);
    // The size counts UTF-16 units, as the reference's document does; a
    // character is never split.
    let mut c = Console::with_cycle_buffer(Some(4));
    c.push(Normal, "ab😀cd".as_bytes());
    assert_eq!(lines(&c), ["😀cd"]);
    assert_eq!(c.len(), 4);
    c.set_cycle_buffer(Some(3));
    assert_eq!(lines(&c), ["cd"]);
    c.set_cycle_buffer(None);
    c.push(Normal, &[b'x'; 5000]);
    assert_eq!(c.len(), 5002);
}

#[test]
fn cycle_buffer_keeps_runs_aligned() {
    let mut c = Console::with_cycle_buffer(Some(5));
    c.push(Normal, b"abc\x1b[31mdef");
    let line = c.line(0).unwrap();
    assert_eq!(line.text(), "bcdef");
    let runs: Vec<_> = line.runs().iter().map(|r| r.range.clone()).collect();
    assert_eq!(runs, [0..2, 2..5]);
}

struct Word(&'static str);

impl LinkFilter for Word {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        for (i, _) in line.match_indices(self.0) {
            out.push(Link {
                range: i..i + self.0.len(),
                target: LinkTarget::Url(self.0.to_owned()),
            });
        }
    }
}

#[test]
fn links_follow_changed_lines() {
    let mut c = Console::new();
    c.set_link_filters(vec![Arc::new(Word("LINK")), Arc::new(UrlFilter)]);
    c.push(Normal, b"a LINK https://example.org/x\nLI");
    let ranges: Vec<_> = c
        .line(0)
        .unwrap()
        .links()
        .iter()
        .map(|l| l.range.clone())
        .collect();
    assert_eq!(ranges, [2..6, 7..28]);
    assert!(c.line(1).unwrap().links().is_empty());
    // The current partial line is filtered again when it changes.
    c.push(Normal, b"NK");
    assert_eq!(c.line(1).unwrap().links().len(), 1);
    c.push(Normal, b"\r");
    c.push(Normal, b"x");
    assert!(c.line(1).unwrap().links().is_empty());
}

/// Appending megabytes stays fast: decoding, line splitting and link
/// filtering do not allocate per byte.
#[test]
fn appending_megabytes_is_fast() {
    let mut chunk = Vec::new();
    while chunk.len() < 8192 * 4 {
        chunk.extend_from_slice(
            b"\x1b[32mINFO\x1b[0m 2026-10-13 12:00:00 request /api/v1/items took 12 ms\n",
        );
        chunk.extend_from_slice(
            "Traceback (most recent call last): see https://example.org/a ✓\n".as_bytes(),
        );
    }
    let mut c = Console::new();
    c.set_link_filters(vec![Arc::new(UrlFilter)]);
    let started = Instant::now();
    let mut total = 0;
    while total < 8 * 1024 * 1024 {
        for piece in chunk.chunks(8192) {
            c.push(Normal, piece);
            total += piece.len();
        }
        c.take_changes();
    }
    let elapsed = started.elapsed();
    assert!(c.len() <= console::DEFAULT_CYCLE_BUFFER_SIZE);
    assert!(elapsed < Duration::from_secs(20), "8 MB took {elapsed:?}");
}
