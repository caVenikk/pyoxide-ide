//! Python completion items as the reference presents them: the parameters
//! in grey right after the name (`check(self, value)`), and at the right
//! edge the class the member belongs to (`Point`), or the type of an
//! attribute when no class is known.
//!
//! The type servers say it in their own words: ty in each item's `detail`
//! (`bound method Point.check(value: int | None = None) -> None`, `def
//! parse(text: str, *, strict: bool = False) -> Point`, `float`),
//! basedpyright only in the documentation of a resolved item (a
//! ```` ```python ```` block: `def check(value: int | None = None) ->
//! None`, `x: float`). The full description stays in `detail` and the
//! documentation.
//!
//! The parameters are their names only, without annotations and defaults,
//! with `*`, `/`, `*args` and `**kwargs` as written; a bound method starts
//! with `self`, one bound to the class (a class method) with `cls`.

use code_intel::{CompletionItem, SymbolKind};

/// Sets the tail and type text of an item from its description, when the
/// description is one of the forms above; other items stay as they are.
/// A tail the server gave (ty's ` (import collections)` of an item that
/// adds an import) is kept.
pub fn present(item: &mut CompletionItem) {
    let Some(description) = description(item) else {
        return;
    };
    let Some((tail, type_text)) = describe(&item.label, item.kind, &description) else {
        return;
    };
    if item.tail_text.is_none() {
        item.tail_text = tail;
    }
    item.type_text = type_text;
}

/// The description of an item: its `detail`, else the first ```` ```python
/// ```` block of its documentation on one line.
fn description(item: &CompletionItem) -> Option<String> {
    if let Some(detail) = item.detail.as_deref().filter(|d| !d.trim().is_empty()) {
        return Some(detail.trim().to_owned());
    }
    let text = &item.documentation.as_ref()?.text;
    let block = text.split("```python").nth(1)?;
    let block = &block[..block.find("```")?];
    let line = block
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!line.is_empty()).then_some(line)
}

/// The tail and type text of an item from its description; `None` when the
/// form is not known.
fn describe(
    label: &str,
    kind: Option<SymbolKind>,
    description: &str,
) -> Option<(Option<String>, Option<String>)> {
    if let Some(rest) = description.strip_prefix("bound method ") {
        let call = format!(".{label}(");
        let at = rest.find(&call)?;
        let owner = &rest[..at];
        let (owner, first) = match owner
            .strip_prefix("type[")
            .and_then(|o| o.strip_suffix(']'))
        {
            Some(class) => (class, "cls"),
            None => (owner, "self"),
        };
        let params = parameters(&rest[at + call.len() - 1..])?;
        return Some((Some(tail(Some(first), &params)), Some(class_name(owner))));
    }
    let function = description
        .strip_prefix("async def ")
        .or_else(|| description.strip_prefix("def "));
    if let Some(rest) = function {
        let params = parameters(rest.strip_prefix(label)?)?;
        // basedpyright's methods are bound ones without a word for it.
        let first = (kind == Some(SymbolKind::Method)).then_some("self");
        return Some((Some(tail(first, &params)), None));
    }
    if description.starts_with("class ")
        || description.starts_with("<class '")
        || description.starts_with("<module '")
    {
        return Some((None, None));
    }
    let attribute = matches!(
        kind,
        Some(
            SymbolKind::Field
                | SymbolKind::Variable
                | SymbolKind::Property
                | SymbolKind::Constant
                | SymbolKind::EnumMember
                | SymbolKind::Value
        )
    );
    if attribute {
        let type_text = description
            .strip_prefix(label)
            .and_then(|rest| rest.strip_prefix(':'))
            .unwrap_or(description)
            .trim();
        let type_text = type_text
            .strip_suffix("(property)")
            .unwrap_or(type_text)
            .trim();
        return Some((None, (!type_text.is_empty()).then(|| type_text.to_owned())));
    }
    None
}

/// `(self, value)`.
fn tail(first: Option<&str>, params: &[String]) -> String {
    let names: Vec<&str> = first
        .into_iter()
        .chain(params.iter().map(String::as_str))
        .collect();
    format!("({})", names.join(", "))
}

/// The class name of an owner (`list[int]` is `list`,
/// `collections.OrderedDict` is `OrderedDict`).
fn class_name(owner: &str) -> String {
    let owner = &owner[..owner.find('[').unwrap_or(owner.len())];
    owner.rsplit('.').next().unwrap_or(owner).trim().to_owned()
}

/// The parameter names of a parenthesized list at the start of `text`
/// (`(a, /, b: int = 1, *args, **kwargs) -> None`).
fn parameters(text: &str) -> Option<Vec<String>> {
    let inner = parenthesized(text)?;
    Some(
        split_top_level(inner)
            .into_iter()
            .map(str::trim)
            .filter(|p| !p.is_empty() && *p != "...")
            .map(|p| {
                let end = top_level_find(p, &[':', '=']).unwrap_or(p.len());
                p[..end].trim().to_owned()
            })
            .collect(),
    )
}

/// The text inside the parentheses `text` starts with.
fn parenthesized(text: &str) -> Option<&str> {
    let text = text.strip_prefix('(')?;
    let mut depth = 0usize;
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')') if depth == 0 => return Some(&text[..i]),
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// `text` split at the commas outside brackets and quotes.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut rest = text;
    while let Some(at) = top_level_find(rest, &[',']) {
        parts.push(&text[start..start + at]);
        start += at + 1;
        rest = &text[start..];
    }
    parts.push(&text[start..]);
    parts
}

/// The first of `chars` outside brackets and quotes.
fn top_level_find(text: &str, chars: &[char]) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            (None, c) if depth == 0 && chars.contains(&c) => return Some(i),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use code_intel::{Markup, MarkupKind};

    use super::*;

    /// An item as ty sends it: the description in `detail` and in
    /// `labelDetails.description` (the type text).
    fn ty(label: &str, kind: SymbolKind, detail: &str) -> CompletionItem {
        let mut item = CompletionItem::new(label).with_kind(kind);
        item.detail = Some(detail.into());
        item.type_text = Some(detail.into());
        item
    }

    /// An item of basedpyright once resolved: the description in its
    /// documentation.
    fn basedpyright(label: &str, kind: SymbolKind, documentation: &str) -> CompletionItem {
        let mut item = CompletionItem::new(label).with_kind(kind);
        item.documentation = Some(Markup {
            kind: MarkupKind::Markdown,
            text: documentation.into(),
        });
        item
    }

    /// The tail and type text of an item as presented.
    fn shown(mut item: CompletionItem) -> (Option<String>, Option<String>) {
        present(&mut item);
        (item.tail_text, item.type_text)
    }

    fn pair(tail: Option<&str>, type_text: Option<&str>) -> (Option<String>, Option<String>) {
        (tail.map(str::to_owned), type_text.map(str::to_owned))
    }

    #[test]
    fn ty_items() {
        use SymbolKind::*;
        // Members of `self.` / `p.`, as ty 0.0.87 describes them.
        let cases = [
            (
                ty(
                    "check",
                    Method,
                    "bound method Point.check(value: int | None = None) -> None",
                ),
                pair(Some("(self, value)"), Some("Point")),
            ),
            (
                ty("norm", Method, "bound method Point.norm() -> float"),
                pair(Some("(self)"), Some("Point")),
            ),
            (
                ty(
                    "origin",
                    Method,
                    "bound method type[Point].origin() -> Point",
                ),
                pair(Some("(cls)"), Some("Point")),
            ),
            (
                ty(
                    "parse",
                    Function,
                    "def parse(text: str, *, strict: bool = False) -> Point",
                ),
                pair(Some("(text, *, strict)"), None),
            ),
            (ty("x", Field, "float"), pair(None, Some("float"))),
            (ty("size", Field, "float"), pair(None, Some("float"))),
            (
                ty("__doc__", Field, "str | None"),
                pair(None, Some("str | None")),
            ),
            (
                ty(
                    "__eq__",
                    Method,
                    "bound method Point.__eq__(value: object, /) -> bool",
                ),
                pair(Some("(self, value, /)"), Some("Point")),
            ),
            (
                ty(
                    "__init__",
                    Method,
                    "bound method Point.__init__(x: float, y: float) -> None",
                ),
                pair(Some("(self, x, y)"), Some("Point")),
            ),
            (
                ty(
                    "free",
                    Function,
                    "def free(a, /, b, *args, c=1, **kwargs) -> Unknown",
                ),
                pair(Some("(a, /, b, *args, c, **kwargs)"), None),
            ),
            (
                ty("frozenset", Class, "<class 'frozenset'>"),
                pair(None, None),
            ),
            (
                ty(
                    "append",
                    Method,
                    "bound method list[int].append(object: int, /) -> None",
                ),
                pair(Some("(self, object, /)"), Some("list")),
            ),
            (
                ty(
                    "get",
                    Method,
                    "bound method dict[str, tuple[int, int]].get(key: str, default: tuple[int, int] = (0, 0), /) -> tuple[int, int]",
                ),
                pair(Some("(self, key, default, /)"), Some("dict")),
            ),
        ];
        for (item, want) in cases {
            let label = item.label.clone();
            assert_eq!(shown(item), want, "{label}");
        }
        // A form not known stays as the server sent it.
        let class = ty("__class__", Class, "type[Point]");
        assert_eq!(shown(class), pair(None, Some("type[Point]")));
        // The full description stays for the documentation.
        let mut check = ty(
            "check",
            Method,
            "bound method Point.check(value: int | None = None) -> None",
        );
        present(&mut check);
        assert_eq!(
            check.detail.as_deref(),
            Some("bound method Point.check(value: int | None = None) -> None")
        );
    }

    #[test]
    fn a_tail_of_the_server_stays() {
        let mut item = ty("OrderedDict", SymbolKind::Class, "<class 'OrderedDict'>");
        item.tail_text = Some(" (import collections)".into());
        present(&mut item);
        assert_eq!(item.tail_text.as_deref(), Some(" (import collections)"));
        assert_eq!(item.type_text, None);
    }

    #[test]
    fn basedpyright_items() {
        use SymbolKind::*;
        // As basedpyright 1.40.2 resolves them.
        let cases = [
            (
                basedpyright(
                    "check",
                    Method,
                    "```python\ndef check(value: int | None = None) -> None\n```",
                ),
                pair(Some("(self, value)"), None),
            ),
            (
                basedpyright(
                    "parse",
                    Method,
                    "```python\ndef parse(\n    text: str,\n    *,\n    strict: bool = False\n) -> Point\n```",
                ),
                pair(Some("(self, text, *, strict)"), None),
            ),
            (
                basedpyright("x", Variable, "```python\nx: float\n```"),
                pair(None, Some("float")),
            ),
            (
                basedpyright("size", Property, "```python\nsize: float (property)\n```"),
                pair(None, Some("float")),
            ),
            (
                basedpyright(
                    "__eq__",
                    Method,
                    "```python\ndef __eq__(\n    value: object,\n    /\n) -> bool\n```\n---\nReturn self==value.",
                ),
                pair(Some("(self, value, /)"), None),
            ),
            (
                basedpyright(
                    "free",
                    Function,
                    "```python\ndef free(\n    a: Unknown,\n    /,\n    b: Unknown,\n    ...,\n    c: int = 1\n) -> None\n```",
                ),
                pair(Some("(a, /, b, c)"), None),
            ),
            (
                basedpyright(
                    "frozenset",
                    Class,
                    "```python\nclass frozenset()\n```\n---\nBuild an immutable unordered collection of unique elements.",
                ),
                pair(None, None),
            ),
        ];
        for (item, want) in cases {
            let label = item.label.clone();
            assert_eq!(shown(item), want, "{label}");
        }
        // Before it is resolved an item has nothing to say.
        let bare = CompletionItem::new("norm").with_kind(Method);
        assert_eq!(shown(bare), pair(None, None));
    }

    #[test]
    fn parameter_lists() {
        assert_eq!(
            parenthesized("(a, (b, c), \")\") -> x"),
            Some("a, (b, c), \")\"")
        );
        assert_eq!(parenthesized("(unclosed"), None);
        assert_eq!(
            parameters("(a: dict[str, int] = {'k': 1}, *, b: str = 'x, y')").unwrap(),
            ["a", "*", "b"]
        );
        assert_eq!(parameters("()").unwrap(), Vec::<String>::new());
        assert_eq!(
            class_name("collections.OrderedDict[str, int]"),
            "OrderedDict"
        );
    }
}
