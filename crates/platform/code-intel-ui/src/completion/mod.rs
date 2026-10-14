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

/// How the typed prefix matches names, as the reference's completion
/// matcher does (`CamelHumpMatcher` with its default settings): name
/// matching from any word of the name (`*` before the prefix, "middle
/// matching"), the case of the first letter counting ("first letter" case
/// sensitivity), and leading underscores skipped where that first letter
/// is concerned.
pub struct PrefixMatcher {
    prefix: String,
    matcher: Matcher,
    /// The same without regard to case: whether a match is at the start.
    any_case: Matcher,
}

impl PrefixMatcher {
    pub fn new(prefix: &str) -> Self {
        let pattern = format!("*{prefix}");
        Self {
            prefix: prefix.to_owned(),
            matcher: Matcher::new(&pattern, MatchingMode::FirstLetter),
            any_case: Matcher::new(&pattern, MatchingMode::IgnoreCase),
        }
    }

    /// The chars of `name` the prefix matches, `None` when it does not
    /// match. A name that starts with `_` does not match when its first
    /// letter after the underscores and the prefix's differ in case.
    pub fn matches(&self, name: &str) -> Option<Vec<Range<usize>>> {
        if self.prefix.is_empty() {
            return Some(Vec::new());
        }
        if name.starts_with('_') && first_letter_case_differs(name, &self.prefix) {
            return None;
        }
        self.matcher.find(name)
    }

    /// The match starts with the name (after its leading underscores):
    /// such names come first.
    pub fn is_start_match(&self, name: &str) -> bool {
        if self.prefix.is_empty() {
            return true;
        }
        match self.any_case.find(name) {
            None => false,
            Some(fragments) => fragments
                .first()
                .is_none_or(|f| f.start <= leading_underscores(name)),
        }
    }
}

fn leading_underscores(name: &str) -> usize {
    name.chars().take_while(|c| *c == '_').count()
}

fn first_letter_case_differs(name: &str, prefix: &str) -> bool {
    let first = |s: &str| s.chars().find(|c| *c != '_');
    match (first(name), first(prefix)) {
        (Some(a), Some(b)) => {
            a.is_lowercase() != b.is_lowercase() || a.is_uppercase() != b.is_uppercase()
        }
        _ => false,
    }
}

/// The items that match `prefix`, in the order the popup shows them, as
/// the reference's sorter orders server items: names that start with the
/// prefix (leading underscores aside) first, then the others; in each
/// group by the server's sort text (items without one after those with
/// one), then names that start with the prefix exactly as typed, then the
/// order of the answer.
pub fn filter(items: &[CompletionItem], prefix: &str) -> Vec<Shown> {
    let matcher = PrefixMatcher::new(prefix);
    let mut shown: Vec<(bool, Option<&str>, bool, Shown)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let name = item.filter_text();
            matcher.matches(name)?;
            // The matched chars are drawn on the label.
            let matched = matcher.matches(&item.label).unwrap_or_default();
            Some((
                matcher.is_start_match(name),
                item.sort_text.as_deref(),
                !prefix.is_empty() && name.starts_with(prefix),
                Shown { item: i, matched },
            ))
        })
        .collect();
    shown.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| match (a.1, b.1) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.item.cmp(&b.3.item))
    });
    shown.into_iter().map(|(_, _, _, s)| s).collect()
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
    /// The completed name follows a `.`: values are attributes (their
    /// icon).
    pub member: bool,
    /// The selection is the person's: Basic Completion, or a key moved
    /// it. An automatic popup starts with the first item preselected only
    /// (the reference's semi-focused lookup).
    pub focused: bool,
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
            member: false,
            focused: explicit,
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
            "focused": self.focused,
            "shown": shown,
            "pending": self.request.is_some(),
            "incomplete": self.incomplete,
            "selected": self.selected,
            "items": self.shown.iter().map(|s| {
                let item = &self.items[s.item];
                json!({
                    "label": item.label,
                    "kind": item.kind,
                    "icon": list::kind_icon(item.kind, self.member).map(|i| i.0),
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

/// Parentheses after functions and methods, as a language's insert
/// handler adds them (the reference's `PyFunctionInsertHandler` on top of
/// `ParenthesesInsertHandler`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parentheses {
    /// Names of a method's first parameter that a call does not pass
    /// (`self`, `cls`): a method with only that parameter takes none.
    pub implicit_first: &'static [&'static str],
}

/// The edits that insert `item` and where the caret goes after them.
///
/// The item replaces the typed prefix (from `start` to the caret) with
/// Enter, or the whole identifier at the caret with Tab (`replace`); an
/// item with its own range replaces from that range's start. A snippet
/// loses its placeholders and the caret goes to its first one. With
/// `parentheses`, a function or method gets `()` unless a `(` follows
/// already, the caret between them when it takes parameters, after them
/// when not. Its additional edits (an import) go into the same change.
/// Ranges of the item are in the text of `version` and are moved to the
/// current text.
#[allow(clippy::too_many_arguments)]
pub fn insertion(
    item: &CompletionItem,
    versions: &Versions,
    version: u64,
    buffer: &Buffer,
    start: usize,
    caret: usize,
    replace: bool,
    parentheses: Option<Parentheses>,
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
    let (mut text, snippet_caret) = if item.snippet {
        snippet(text)
    } else {
        (text.to_owned(), None)
    };
    let tail = range.start + text.chars().count();
    // Positions after the replaced range, in the text after the edit.
    let after = |old: usize| old - range.end + tail;
    let mut caret_after = snippet_caret.map_or(tail, |c| range.start + c);
    let callable = matches!(item.kind, Some(SymbolKind::Function | SymbolKind::Method));
    if let Some(parens) = parentheses.filter(|_| callable && !item.snippet && !text.contains('(')) {
        let inside = has_parameters(item, parens);
        let mut next = range.end;
        while matches!(buffer.char_at(next), Some(' ' | '\t')) {
            next += 1;
        }
        if buffer.char_at(next) == Some('(') {
            // Parentheses are there: Tab goes into them, Enter stays after
            // the name, unless the function takes parameters or they hold
            // nothing.
            caret_after = if replace || inside {
                after(next + 1)
            } else {
                tail
            };
            if let Some(close) = closing_parenthesis(buffer, next) {
                let empty = buffer.slice(next + 1..close).trim().is_empty();
                if !inside && empty {
                    caret_after = after(close + 1);
                } else if inside {
                    caret_after = after(next + 1);
                }
            }
        } else {
            text.push_str("()");
            caret_after = tail + if inside { 1 } else { 2 };
        }
    }
    let mut caret_after = caret_after as isize;
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
            caret_after += extra.text.chars().count() as isize - r.len() as isize;
        }
        edits.push(Edit::new(r, extra.text.clone()));
    }
    (edits, caret_after.max(0) as usize)
}

/// Whether a function or method item takes parameters, from the first
/// parameter list in its detail, type or tail text (a method's implicit
/// first parameter does not count); without one, it is taken to have some.
pub fn has_parameters(item: &CompletionItem, parens: Parentheses) -> bool {
    let texts = [&item.detail, &item.type_text, &item.tail_text];
    let Some(params) = texts
        .into_iter()
        .flatten()
        .find_map(|text| parameter_list(text))
    else {
        return true;
    };
    let mut params: Vec<&str> = split_top_level(params)
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "/")
        .collect();
    if item.kind == Some(SymbolKind::Method)
        && let Some(first) = params.first()
    {
        let name = first.split([':', '=']).next().unwrap_or_default().trim();
        if parens.implicit_first.contains(&name) {
            params.remove(0);
        }
    }
    !params.is_empty()
}

/// The text between the first `(` of `text` and its `)`; a tail such as
/// ` (import os)` is not a parameter list.
fn parameter_list(text: &str) -> Option<&str> {
    let open = text.find('(')?;
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &text[open + 1..open + i];
                    return (!inner.trim_start().starts_with("import ")).then_some(inner);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parts of a parameter list separated by commas outside brackets.
fn split_top_level(params: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut from) = (0i32, 0);
    for (i, c) in params.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&params[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&params[from..]);
    parts
}

/// The `)` that closes the `(` at `open`, looked for in the rest of the
/// text (not further than a few thousand chars).
fn closing_parenthesis(buffer: &Buffer, open: usize) -> Option<usize> {
    let mut depth = 0;
    let end = buffer.len_chars().min(open + 4096);
    for pos in open..end {
        match buffer.char_at(pos)? {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => {}
        }
    }
    None
}

/// The text of a snippet without its placeholders' markup (`${1:name}`
/// becomes `name`, `$1` and `$0` go), and the char offset of its first
/// placeholder (the lowest number, `$0` last), where the caret goes.
pub fn snippet(snippet: &str) -> (String, Option<usize>) {
    let mut out = String::new();
    let mut len = 0;
    // (number, offset) of the first placeholder found so far.
    let mut first: Option<(u32, usize)> = None;
    let stop = |number: u32, at: usize, first: &mut Option<(u32, usize)>| {
        let rank = if number == 0 { u32::MAX } else { number };
        if first.is_none_or(|(r, _)| rank < r) {
            *first = Some((rank, at));
        }
    };
    let mut chars = snippet.chars().peekable();
    let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
        let mut n = 0u32;
        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d);
            chars.next();
        }
        n
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                    len += 1;
                }
            }
            '$' if chars.peek().is_some_and(|c| c.is_ascii_digit()) => {
                let n = number(&mut chars);
                stop(n, len, &mut first);
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                let n = number(&mut chars);
                stop(n, len, &mut first);
                if chars.peek() == Some(&':') {
                    chars.next();
                }
                let mut depth = 0;
                for c in chars.by_ref() {
                    match c {
                        '}' if depth == 0 => break,
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    out.push(c);
                    len += 1;
                }
            }
            c => {
                out.push(c);
                len += 1;
            }
        }
    }
    (out, first.map(|(_, at)| at))
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
        // Members of `self` after `self.` as a server sorts them (own ones,
        // then the inherited dunders); typing `n` gives the reference's
        // order: `norm`, `__ne__` and `__new__` start with it (underscores
        // aside), the others match inside.
        let mut items = items(&[
            "norm",
            "x",
            "__annotations__",
            "__init__",
            "__init_subclass__",
            "__ne__",
            "__new__",
        ]);
        for (i, item) in items.iter_mut().enumerate() {
            item.sort_text = Some(format!("{i:02}"));
        }
        // Nothing typed: everything, in the server's order.
        let all = filter(&items, "");
        assert_eq!(labels(&items, &all).len(), 7);
        assert!(all.iter().all(|s| s.matched.is_empty()));
        let shown = filter(&items, "n");
        assert_eq!(
            labels(&items, &shown),
            [
                "norm",
                "__ne__",
                "__new__",
                "__annotations__",
                "__init__",
                "__init_subclass__"
            ]
        );
        assert_eq!(shown[0].matched, vec![0..1]);
        assert_eq!(shown[1].matched, vec![2..3]);
        // The first letter's case counts.
        assert!(filter(&items, "N").is_empty());
        let names = self::items(&["Path", "path", "_Private", "_private"]);
        // (An upper-case first letter is compared with the name's first
        // char, the underscore, as the reference's matcher does.)
        assert_eq!(labels(&names, &filter(&names, "P")), ["Path"]);
        assert_eq!(labels(&names, &filter(&names, "p")), ["path", "_private"]);
        assert_eq!(labels(&names, &filter(&names, "pa")), ["path"]);
        // Without sort texts: the answer's order, exact-case starts first.
        let plain = self::items(&["b", "aB", "Ab", "ab"]);
        assert_eq!(labels(&plain, &filter(&plain, "")), ["b", "aB", "Ab", "ab"]);
        assert_eq!(labels(&plain, &filter(&plain, "ab")), ["ab", "aB"]);
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
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 2, 4, false, None);
        assert_eq!(edits, vec![Edit::new(2..4, "norm")]);
        assert_eq!(caret, 6);
        // Tab: the whole identifier goes.
        let (edits, _) = insertion(&item, &versions, 1, &buffer, 2, 4, true, None);
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
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 4, 6, false, None);
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
        let (edits, caret) = insertion(&item, &versions, 1, &buffer, 5, 7, false, None);
        assert_eq!(edits, vec![Edit::new(5..7, "norm")]);
        assert_eq!(caret, 9);
    }

    #[test]
    fn snippets_lose_their_markup() {
        assert_eq!(
            snippet("print(${1:value})$0"),
            ("print(value)".into(), Some(6))
        );
        assert_eq!(snippet("f($2, $1)"), ("f(, )".into(), Some(4)));
        assert_eq!(snippet("x$0"), ("x".into(), Some(1)));
        assert_eq!(snippet(r"a\$b"), ("a$b".into(), None));
        assert_eq!(snippet("${1:{x}}"), ("{x}".into(), Some(0)));
    }

    const PYTHON: Option<Parentheses> = Some(Parentheses {
        implicit_first: &["self", "cls"],
    });

    /// Inserts `item` with Enter (or Tab) at `|` in `marked`, the typed
    /// prefix from `start`; the text after, `|` at the caret.
    fn insert(item: &CompletionItem, marked: &str, start: usize, replace: bool) -> String {
        let caret = marked.find('|').unwrap();
        let mut buffer = Buffer::new(&marked.replace('|', ""));
        let versions = Versions::new(buffer.rope().clone());
        let (edits, caret) = insertion(item, &versions, 1, &buffer, start, caret, replace, PYTHON);
        let mut selections = text::SelectionSet::caret(0);
        buffer.transact(
            &mut selections,
            edits,
            text::EditKind::Other,
            text::CaretPlacement::AfterEachEdit,
        );
        let mut text = buffer.text();
        text.insert(caret, '|');
        text
    }

    fn function(label: &str, kind: SymbolKind, detail: Option<&str>) -> CompletionItem {
        let mut item = CompletionItem::new(label).with_kind(kind);
        item.detail = detail.map(Into::into);
        item
    }

    #[test]
    fn functions_get_parentheses_as_the_python_insert_handler_adds_them() {
        let norm = function(
            "norm",
            SymbolKind::Method,
            Some("bound method Point.norm() -> float"),
        );
        let scaled = function(
            "scaled",
            SymbolKind::Method,
            Some("def scaled(self, k: float) -> Point"),
        );
        let print = function("print", SymbolKind::Function, None);
        let field = function("x", SymbolKind::Field, Some("int"));
        // No parameters: the caret after `()`; parameters (besides `self`):
        // between them; nothing known: between them.
        assert_eq!(insert(&norm, "p.no|", 2, false), "p.norm()|");
        assert_eq!(insert(&scaled, "p.sc|", 2, false), "p.scaled(|)");
        assert_eq!(insert(&print, "pr|", 0, false), "print(|)");
        assert_eq!(insert(&field, "p.|", 2, false), "p.x|");
        // A `(` follows already: no new one; Enter stays after the name,
        // or goes after empty parentheses; Tab goes into them.
        assert_eq!(insert(&scaled, "p.sc|(2)", 2, false), "p.scaled(|2)");
        assert_eq!(insert(&norm, "p.no|()", 2, false), "p.norm()|");
        assert_eq!(insert(&norm, "p.no| (x)", 2, false), "p.norm| (x)");
        assert_eq!(insert(&norm, "p.no|rm(x)", 2, true), "p.norm(|x)");
        // Snippets and texts with their own parentheses are left alone.
        let mut call = function("f", SymbolKind::Function, None);
        call.insert = InsertText::Text("f(${1:a})".into());
        call.snippet = true;
        assert_eq!(insert(&call, "|", 0, false), "f(|a)");
    }

    #[test]
    fn parameters_from_the_servers_texts() {
        let mut item = function("f", SymbolKind::Method, None);
        let has = |item: &CompletionItem| has_parameters(item, PYTHON.unwrap());
        assert!(has(&item), "nothing known");
        item.type_text = Some("(self) -> None".into());
        assert!(!has(&item));
        item.type_text = Some("(cls, /, x: dict[str, int] = {}) -> None".into());
        assert!(has(&item));
        item.type_text = None;
        item.tail_text = Some(" (import os)".into());
        assert!(has(&item), "an import tail is not a parameter list");
        item.detail = Some("def f(self, /)".into());
        assert!(!has(&item));
    }
}
