//! Code completion (the reference's lookup): a session per popup, the
//! filtering and ordering of the items on the client, and the edits that
//! insert the chosen item.

pub mod list;

use std::ops::Range;
use std::path::PathBuf;

use code_intel::{CompletionItem, InsertText, SymbolKind};
use editor::Editor;
use gpui::{Entity, Task};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use text::{Buffer, Edit};

use crate::document::Versions;

/// The key context of the completion popup.
pub const CONTEXT: &str = "Completion";

/// A char of an identifier: what the typed prefix is made of (letters,
/// digits and `_`, as the reference's auto-popup handler counts them).
pub fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the identifier that ends at `offset`.
pub fn identifier_start(buffer: &Buffer, offset: usize) -> usize {
    let mut start = offset;
    while start > 0 && buffer.char_at(start - 1).is_some_and(is_identifier_char) {
        start -= 1;
    }
    start
}

/// The end of the identifier that continues at `offset`.
pub fn identifier_end(buffer: &Buffer, offset: usize) -> usize {
    let mut end = offset;
    while buffer.char_at(end).is_some_and(is_identifier_char) {
        end += 1;
    }
    end
}

/// An item shown in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Its index in the answer.
    pub item: usize,
    /// The chars of its label the prefix matched.
    pub matched: Vec<Range<usize>>,
}

/// The matcher of a typed prefix: name matching that may also start in the
/// middle of a name (`*` first), as the reference's completion matches.
fn matcher(prefix: &str) -> Matcher {
    Matcher::new(&format!("*{prefix}"), MatchingMode::IgnoreCase).prefer_start()
}

/// The items that match `prefix`, in the order the popup shows them:
/// items whose name starts with the prefix first, then the others; in each
/// group the server's order (its sort text, then the order of the answer).
pub fn filter(items: &[CompletionItem], prefix: &str) -> Vec<Shown> {
    let matcher = (!prefix.is_empty()).then(|| matcher(prefix));
    let mut shown: Vec<(bool, &str, Shown)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let filter: Vec<char> = item.filter_text().chars().collect();
            let (start, fragments) = match &matcher {
                None => (true, Vec::new()),
                Some(m) => {
                    let found = m.match_chars(&filter)?;
                    (found.is_start_match(), found.fragments)
                }
            };
            // Highlights are drawn on the label: only when the filter text
            // is the label.
            let matched = if item.filter_text.is_none() || item.filter_text() == item.label {
                fragments
            } else {
                Vec::new()
            };
            Some((start, item.sort_text(), Shown { item: i, matched }))
        })
        .collect();
    shown.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.item.cmp(&b.2.item))
    });
    shown.into_iter().map(|(_, _, s)| s).collect()
}

/// A completion popup and the requests behind it.
pub struct Session {
    pub id: u64,
    /// The document (its path).
    pub path: PathBuf,
    pub editor: Entity<Editor>,
    /// Where the identifier being completed starts, in the current text.
    pub start: usize,
    /// Asked for by Basic Completion rather than by typing.
    pub explicit: bool,
    /// The items of the last answer, and the version of the text they are
    /// in.
    pub items: Vec<CompletionItem>,
    pub answer_version: u64,
    /// The server filtered the items by the prefix it saw.
    pub incomplete: bool,
    /// The prefix the shown items were filtered with.
    pub prefix: String,
    /// The prefix of the request in flight.
    pub requested_prefix: String,
    pub shown: Vec<Shown>,
    pub selected: usize,
    /// The request in flight: dropping it cancels the request.
    pub request: Option<Task<()>>,
    /// An answer came.
    pub answered: bool,
}

impl Session {
    pub fn new(
        id: u64,
        path: PathBuf,
        editor: Entity<Editor>,
        start: usize,
        explicit: bool,
    ) -> Self {
        Self {
            id,
            path,
            editor,
            start,
            explicit,
            items: Vec::new(),
            answer_version: 0,
            incomplete: false,
            prefix: String::new(),
            requested_prefix: String::new(),
            shown: Vec::new(),
            selected: 0,
            request: None,
            answered: false,
        }
    }

    /// Filters the items of the answer with a prefix and selects the first
    /// one (with nothing typed, the item the server preselects).
    pub fn refilter(&mut self, prefix: &str) {
        self.shown = filter(&self.items, prefix);
        self.prefix = prefix.to_owned();
        self.selected = self
            .shown
            .iter()
            .position(|s| self.items[s.item].preselect)
            .filter(|_| prefix.is_empty())
            .unwrap_or(0);
    }

    /// The index in the answer of the selected item.
    pub fn selected_item(&self) -> Option<usize> {
        self.shown.get(self.selected).map(|s| s.item)
    }

    /// Moves the selection by `delta` rows; past an end it goes round to
    /// the other end when `wrap`, else it stops there.
    pub fn select_by(&mut self, delta: isize, wrap: bool) {
        self.selected = moved(self.selected, self.shown.len(), delta, wrap);
    }

    pub fn select(&mut self, index: usize) {
        if index < self.shown.len() {
            self.selected = index;
        }
    }

    /// The state reported by `code.state`.
    pub fn state_json(&self, shown: bool) -> Json {
        json!({
            "prefix": self.prefix,
            "explicit": self.explicit,
            "shown": shown,
            "pending": self.request.is_some(),
            "incomplete": self.incomplete,
            "selected": self.selected,
            "items": self.shown.iter().map(|s| {
                let item = &self.items[s.item];
                json!({
                    "label": item.label,
                    "kind": item.kind,
                    "tail": item.tail_text,
                    "type": item.type_text,
                    "bold": is_bold(item),
                    "deprecated": item.deprecated,
                    "matched": s.matched.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        })
    }
}

/// A row index moved by `delta` in a list of `len` rows; past an end it
/// goes round to the other end when `wrap`, else it stops there.
pub fn moved(index: usize, len: usize, delta: isize, wrap: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    let next = index as isize + delta;
    if (0..len).contains(&next) {
        next as usize
    } else if wrap {
        next.rem_euclid(len) as usize
    } else {
        next.clamp(0, len - 1) as usize
    }
}

/// Keywords are bold (`LspCompletionCustomizer.isBold`).
pub fn is_bold(item: &CompletionItem) -> bool {
    item.kind == Some(SymbolKind::Keyword)
}

/// The edits that insert `item` and where the caret goes after them.
///
/// The item replaces the typed prefix (from `start` to the caret) with
/// Enter, or the whole identifier at the caret with Tab (`replace`); an
/// item with its own range replaces from that range's start. Its
/// additional edits (an import) go into the same change. Ranges of the
/// item are in the text of `version` and are moved to the current text.
pub fn insertion(
    item: &CompletionItem,
    versions: &Versions,
    version: u64,
    buffer: &Buffer,
    start: usize,
    caret: usize,
    replace: bool,
) -> (Vec<Edit>, usize) {
    let end = if replace {
        identifier_end(buffer, caret)
    } else {
        caret
    };
    let (range, text) = match &item.insert {
        InsertText::Text(text) => (start..end, text.as_str()),
        InsertText::Edit {
            insert,
            replace: replace_range,
            text,
        } => {
            let edit_start = versions
                .offset(version, insert.start)
                .filter(|s| *s <= caret)
                .unwrap_or(start);
            let edit_end = if replace {
                versions
                    .offset(version, replace_range.end)
                    .map_or(end, |e| e.max(end))
            } else {
                caret
            };
            (edit_start..edit_end, text.as_str())
        }
    };
    let text = if item.snippet {
        snippet_text(text)
    } else {
        text.to_owned()
    };
    let mut caret_after = range.start + text.chars().count();
    let mut edits = vec![Edit::new(range.clone(), text)];
    for extra in &item.additional_edits {
        let Some(r) = versions.range(version, extra.range.clone()) else {
            continue;
        };
        // An edit that overlaps the insertion is the server's mistake.
        if r.end > range.start && r.start < range.end {
            continue;
        }
        if r.end <= range.start {
            caret_after = caret_after + extra.text.chars().count() - r.len();
        }
        edits.push(Edit::new(r, extra.text.clone()));
    }
    (edits, caret_after)
}

/// The text of a snippet without its placeholders' markup (`${1:name}`
/// becomes `name`, `$1` and `$0` go).
pub fn snippet_text(snippet: &str) -> String {
    let mut out = String::new();
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' if chars.peek().is_some_and(|c| c.is_ascii_digit()) => {
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
                if chars.peek() == Some(&':') {
                    chars.next();
                }
                let mut depth = 0;
                for c in chars.by_ref() {
                    match c {
                        '{' => {
                            depth += 1;
                            out.push(c);
                        }
                        '}' if depth == 0 => break,
                        '}' => {
                            depth -= 1;
                            out.push(c);
                        }
                        c => out.push(c),
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use code_intel::TextEdit;
    use text::Point;

    use super::*;

    fn items(labels: &[&str]) -> Vec<CompletionItem> {
        labels.iter().map(|l| CompletionItem::new(*l)).collect()
    }

    fn labels(items: &[CompletionItem], shown: &[Shown]) -> Vec<String> {
        shown.iter().map(|s| items[s.item].label.clone()).collect()
    }

    #[test]
    fn identifiers_around_the_caret() {
        let buffer = Buffer::new("self.nor_m(x)");
        assert_eq!(identifier_start(&buffer, 8), 5);
        assert_eq!(identifier_end(&buffer, 8), 10);
        assert_eq!(identifier_start(&buffer, 5), 5);
        assert_eq!(identifier_start(&buffer, 4), 0);
    }

    #[test]
    fn filtering_keeps_the_servers_order_with_start_matches_first() {
        let items = items(&[
            "x",
            "norm",
            "__ne__",
            "__annotations__",
            "scaled",
            "__new__",
        ]);
        // Nothing typed: everything, in the server's order.
        let all = filter(&items, "");
        assert_eq!(labels(&items, &all).len(), 6);
        assert!(all.iter().all(|s| s.matched.is_empty()));
        let shown = filter(&items, "n");
        let names = labels(&items, &shown);
        assert_eq!(names[0], "norm");
        assert!(names.contains(&"__ne__".to_owned()));
        assert!(!names.contains(&"x".to_owned()));
        assert_eq!(shown[0].matched, vec![0..1]);
        // Case does not matter; start matches come first.
        let names = labels(&items, &filter(&items, "NO"));
        assert_eq!(names, ["norm", "__annotations__"]);
        // The sort text decides the order among equal matches.
        let mut sorted = self::items(&["b", "a"]);
        sorted[0].sort_text = Some("0".into());
        sorted[1].sort_text = Some("1".into());
        assert_eq!(labels(&sorted, &filter(&sorted, "")), ["b", "a"]);
    }

    #[test]
    fn selection_moves_and_wraps() {
        assert_eq!(moved(0, 3, 1, true), 1);
        assert_eq!(moved(1, 3, 5, false), 2);
        assert_eq!(moved(2, 3, 1, true), 0);
        assert_eq!(moved(0, 3, -1, true), 2);
        assert_eq!(moved(2, 3, -10, false), 0);
        assert_eq!(moved(0, 0, 1, true), 0);
    }

    #[test]
    fn insertions_replace_the_prefix_or_the_identifier() {
        let buffer = Buffer::new("p.noxyz\n");
        let versions = Versions::new(buffer.rope().clone());
        let item = CompletionItem::new("norm");
        // Enter: the prefix `no` becomes `norm`, the rest stays.
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 2, 4, false);
        assert_eq!(edits, vec![Edit::new(2..4, "norm")]);
        assert_eq!(caret, 6);
        // Tab: the whole identifier goes.
        let (edits, _) = insertion(&item, &versions, 1, &buffer, 2, 4, true);
        assert_eq!(edits, vec![Edit::new(2..7, "norm")]);
    }

    #[test]
    fn additional_edits_go_into_the_same_change() {
        let buffer = Buffer::new("x = Pa\n");
        let versions = Versions::new(buffer.rope().clone());
        let mut item = CompletionItem::new("Path");
        item.additional_edits = vec![TextEdit::insert(
            Point::new(0, 0),
            "from pathlib import Path\n",
        )];
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 4, 6, false);
        assert_eq!(
            edits,
            vec![
                Edit::new(4..6, "Path"),
                Edit::insert(0, "from pathlib import Path\n"),
            ]
        );
        // The caret ends after `Path`, below the new import line.
        assert_eq!(caret, 4 + 4 + 25);
    }

    #[test]
    fn items_with_a_range_of_an_older_text() {
        // The answer was for `self.n` (version 1); `o` was typed since.
        let mut buffer = Buffer::new("self.n\n");
        let mut versions = Versions::new(buffer.rope().clone());
        let mut selections = text::SelectionSet::caret(6);
        let applied = buffer.transact(
            &mut selections,
            vec![Edit::insert(6, "o")],
            text::EditKind::Typing,
            text::CaretPlacement::AfterEachEdit,
        );
        versions.edited(&applied, buffer.rope());
        let mut item = CompletionItem::new("norm");
        let range = Point::new(0, 5)..Point::new(0, 6);
        item.insert = InsertText::Edit {
            insert: range.clone(),
            replace: range,
            text: "norm".into(),
        };
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 5, 7, false);
        assert_eq!(edits, vec![Edit::new(5..7, "norm")]);
        assert_eq!(caret, 9);
    }

    #[test]
    fn snippets_lose_their_markup() {
        assert_eq!(snippet_text("print(${1:value})$0"), "print(value)");
        assert_eq!(snippet_text("f($1, $2)"), "f(, )");
        assert_eq!(snippet_text(r"a\$b"), "a$b");
        assert_eq!(snippet_text("${1:{x}}"), "{x}");
    }
}
