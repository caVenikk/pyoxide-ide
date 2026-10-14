//! Protocol values to the `code-intel` model and back. Lenient: an item of
//! an unknown shape is skipped, not the whole answer.

use std::ops::Range;
use std::path::{Path, PathBuf};

use code_intel::{
    CodeAction, CodeActionKind, Command, CompletionItem, Completions, Diagnostic,
    DocumentHighlight, FileEdits, HighlightKind, Hover, InsertText, Location, Markup,
    ParameterInfo, Point, ServerId, Severity, Signature, SignatureContext, SignatureHelp,
    SignatureTrigger, SymbolKind, TextEdit, Token, WorkspaceEdit,
};
use lsp_types::Position;
use serde_json::{Value, json};

use crate::diagnostics::Item;
use crate::encoding::{PositionEncoding, Text};
use crate::uri;

/// What the client attaches to a model value to find its server and the
/// protocol value again (a diagnostic, a completion item, a code action,
/// a command).
#[derive(Debug, Clone)]
pub(crate) struct Raw {
    pub server: ServerId,
    /// The run of the server that sent it: a restarted server does not
    /// know it.
    pub generation: u64,
    pub value: Value,
}

impl Raw {
    pub fn token(server: &ServerId, generation: u64, value: &Value) -> Token {
        Token::new(Raw {
            server: server.clone(),
            generation,
            value: value.clone(),
        })
    }

    pub fn of(token: Option<&Token>) -> Option<&Raw> {
        token?.downcast_ref::<Raw>()
    }
}

/// Text of the files positions refer to.
pub(crate) type TextOf<'a> = &'a dyn Fn(&Path) -> Result<Text, String>;

/// An LSP position object.
pub(crate) fn position(value: &Value) -> Option<Position> {
    let line = value.get("line")?.as_u64()?;
    let character = value.get("character")?.as_u64()?;
    Some(Position::new(
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(character).unwrap_or(u32::MAX),
    ))
}

/// An LSP range object as char offsets of a text.
pub(crate) fn offsets(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
) -> Option<Range<usize>> {
    let start = text.position_offset(position(value.get("start")?)?, encoding);
    let end = text.position_offset(position(value.get("end")?)?, encoding);
    Some(start.min(end)..start.max(end))
}

/// An LSP range object in a text.
pub(crate) fn range(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
) -> Option<Range<Point>> {
    let offsets = offsets(value, text, encoding)?;
    Some(text.offset_to_point(offsets.start)..text.offset_to_point(offsets.end))
}

/// An LSP range object for a model range.
pub(crate) fn lsp_range(range: &Range<Point>, text: &Text, encoding: PositionEncoding) -> Value {
    json!(text.range(range.clone(), encoding))
}

/// LSP text edits (`TextEdit`, `AnnotatedTextEdit`) in a text.
pub(crate) fn text_edits(value: &Value, text: &Text, encoding: PositionEncoding) -> Vec<TextEdit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|edit| {
            let range = range(edit.get("range")?, text, encoding)?;
            let new_text = edit.get("newText")?.as_str()?;
            Some(TextEdit::new(range, new_text))
        })
        .collect()
}

/// An LSP `WorkspaceEdit` (`changes` or `documentChanges`); `text_of` gives
/// the text of each file the positions refer to. Resource operations
/// (create, rename, delete) are not supported: an edit with them fails
/// as a whole, as `failureHandling: abort` asks.
pub(crate) fn workspace_edit(
    value: &Value,
    encoding: PositionEncoding,
    text_of: TextOf<'_>,
) -> Result<WorkspaceEdit, String> {
    let mut files: Vec<(PathBuf, &Value)> = Vec::new();
    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            if let Some(kind) = change.get("kind").and_then(Value::as_str) {
                return Err(format!("file operations ({kind}) are not supported"));
            }
            let uri = change
                .get("textDocument")
                .and_then(|d| d.get("uri"))
                .and_then(Value::as_str)
                .ok_or("a document change without a document")?;
            let path = uri::str_to_path(uri).ok_or_else(|| format!("not a file: {uri}"))?;
            files.push((path, change.get("edits").unwrap_or(&Value::Null)));
        }
    } else if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let path = uri::str_to_path(uri).ok_or_else(|| format!("not a file: {uri}"))?;
            files.push((path, edits));
        }
    }
    let mut edit = WorkspaceEdit::default();
    for (path, edits) in files {
        let text = text_of(&path)?;
        let edits = text_edits(edits, &text, encoding);
        match edit.files.iter_mut().find(|f| f.path == path) {
            Some(file) => file.edits.extend(edits),
            None => edit.files.push(FileEdits { path, edits }),
        }
    }
    Ok(edit)
}

/// A string or a `MarkupContent` (documentation of items and signatures).
pub(crate) fn markup(value: &Value) -> Option<Markup> {
    let markup = match value {
        Value::String(text) => Markup::plain(text.clone()),
        Value::Object(content) => {
            let text = content.get("value")?.as_str()?.to_owned();
            match content.get("kind").and_then(Value::as_str) {
                Some("markdown") => Markup::markdown(text),
                _ => Markup::plain(text),
            }
        }
        _ => return None,
    };
    (!markup.is_empty()).then_some(markup)
}

/// A diagnostic of a server in the text it was computed for. Severity as
/// ADR 0010 (the reference's `LspDiagnosticsCustomizer`): Error and Warning
/// as they are, Information a weak warning, Hint information (not
/// counted); none at all a weak warning, as the reference does.
pub(crate) fn diagnostic(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
    server: &ServerId,
    generation: u64,
) -> Option<Item> {
    let offsets = offsets(value.get("range")?, text, encoding)?;
    let message = match value.get("message")? {
        Value::String(message) => message.clone(),
        other => other.get("value")?.as_str()?.to_owned(),
    };
    let severity = match value.get("severity").and_then(Value::as_u64) {
        Some(1) => Severity::Error,
        Some(2) => Severity::Warning,
        Some(4) => Severity::Information,
        _ => Severity::WeakWarning,
    };
    let mut diagnostic = Diagnostic::new(
        text.offset_to_point(offsets.start)..text.offset_to_point(offsets.end),
        severity,
        message,
    );
    diagnostic.source = value
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    diagnostic.code = match value.get("code") {
        Some(Value::String(code)) => Some(code.clone()),
        Some(Value::Number(code)) => Some(code.to_string()),
        _ => None,
    };
    diagnostic.code_href = value
        .get("codeDescription")
        .and_then(|d| d.get("href"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let tags: Vec<u64> = value
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| tags.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    diagnostic.unnecessary = tags.contains(&1);
    diagnostic.deprecated = tags.contains(&2);
    diagnostic.server = Some(server.clone());
    diagnostic.token = Some(Raw::token(server, generation, value));
    Some(Item {
        start: offsets.start,
        end: offsets.end,
        diagnostic,
    })
}

fn symbol_kind(kind: u64) -> Option<SymbolKind> {
    use SymbolKind::*;
    Some(match kind {
        1 => Text,
        2 => Method,
        3 => Function,
        4 => Constructor,
        5 => Field,
        6 => Variable,
        7 => Class,
        8 => Interface,
        9 => Module,
        10 => Property,
        11 => Unit,
        12 => Value,
        13 => Enum,
        14 => Keyword,
        15 => Snippet,
        16 => Color,
        17 => File,
        18 => Reference,
        19 => Folder,
        20 => EnumMember,
        21 => Constant,
        22 => Struct,
        23 => Event,
        24 => Operator,
        25 => TypeParameter,
        _ => return None,
    })
}

fn text_of_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A completion answer (`CompletionItem[]`, `CompletionList` or `null`).
pub(crate) fn completions(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
    server: &ServerId,
    generation: u64,
) -> Completions {
    let (items, incomplete) = match value {
        Value::Array(items) => (items.as_slice(), false),
        Value::Object(list) => (
            list.get("items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            list.get("isIncomplete").and_then(Value::as_bool) == Some(true),
        ),
        _ => (&[][..], false),
    };
    Completions {
        items: items
            .iter()
            .filter_map(|item| completion_item(item, text, encoding, server, generation))
            .collect(),
        incomplete,
    }
}

/// One completion item; positions in `text`. Rendered as the reference's
/// `LspCompletionSupport.renderLookupElement`: tail text
/// `labelDetails.detail`, type text `labelDetails.description` or else
/// `detail`.
pub(crate) fn completion_item(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
    server: &ServerId,
    generation: u64,
) -> Option<CompletionItem> {
    let label = value.get("label")?.as_str()?.to_owned();
    let mut item = CompletionItem::new(label.clone());
    item.kind = value
        .get("kind")
        .and_then(Value::as_u64)
        .and_then(symbol_kind);
    let details = value.get("labelDetails").unwrap_or(&Value::Null);
    item.tail_text = text_of_field(details, "detail");
    item.detail = text_of_field(value, "detail");
    item.type_text = text_of_field(details, "description").or_else(|| item.detail.clone());
    item.filter_text = text_of_field(value, "filterText");
    item.sort_text = text_of_field(value, "sortText");
    item.snippet = value.get("insertTextFormat").and_then(Value::as_u64) == Some(2);
    item.insert = match value.get("textEdit") {
        Some(edit) if edit.get("insert").is_some() => {
            let insert = range(edit.get("insert")?, text, encoding)?;
            let replace = range(edit.get("replace")?, text, encoding)?;
            InsertText::Edit {
                insert,
                replace,
                text: edit.get("newText")?.as_str()?.to_owned(),
            }
        }
        Some(edit) if edit.get("range").is_some() => {
            let range = range(edit.get("range")?, text, encoding)?;
            InsertText::Edit {
                insert: range.clone(),
                replace: range,
                text: edit.get("newText")?.as_str()?.to_owned(),
            }
        }
        _ => InsertText::Text(text_of_field(value, "insertText").unwrap_or(label)),
    };
    item.additional_edits = value
        .get("additionalTextEdits")
        .map(|edits| text_edits(edits, text, encoding))
        .unwrap_or_default();
    item.documentation = value.get("documentation").and_then(markup);
    let tags = value.get("tags").and_then(Value::as_array);
    item.deprecated = value.get("deprecated").and_then(Value::as_bool) == Some(true)
        || tags.is_some_and(|tags| tags.iter().any(|t| t.as_u64() == Some(1)));
    item.preselect = value.get("preselect").and_then(Value::as_bool) == Some(true);
    item.server = Some(server.clone());
    item.token = Some(Raw::token(server, generation, value));
    Some(item)
}

/// A hover answer. A plain `MarkedString` is Markdown, a language one a
/// fenced code block; several are separated by empty lines.
pub(crate) fn hover(value: &Value, text: &Text, encoding: PositionEncoding) -> Option<Hover> {
    fn marked(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Object(code) => {
                let language = code.get("language").and_then(Value::as_str)?;
                let value = code.get("value").and_then(Value::as_str)?;
                Some(format!("```{language}\n{value}\n```"))
            }
            _ => None,
        }
    }
    let contents = value.get("contents")?;
    let contents = match contents {
        Value::Object(content) if content.contains_key("kind") => markup(contents)?,
        Value::Array(parts) => Markup::markdown(
            parts
                .iter()
                .filter_map(marked)
                .filter(|p| !p.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        other => Markup::markdown(marked(other)?),
    };
    if contents.is_empty() {
        return None;
    }
    let range = value.get("range").and_then(|r| range(r, text, encoding));
    Some(Hover { contents, range })
}

/// A signature help answer. Parameter labels are byte ranges of the
/// signature's label: a string label is found in it, an offset pair is in
/// the server's encoding.
pub(crate) fn signature_help(value: &Value, encoding: PositionEncoding) -> Option<SignatureHelp> {
    let signatures: Vec<Signature> = value
        .get("signatures")?
        .as_array()?
        .iter()
        .filter_map(|s| signature(s, encoding))
        .collect();
    if signatures.is_empty() {
        return None;
    }
    let active_signature = value
        .get("activeSignature")
        .and_then(Value::as_u64)
        .map_or(0, |a| (a as usize).min(signatures.len() - 1));
    Some(SignatureHelp {
        signatures,
        active_signature,
        active_parameter: value
            .get("activeParameter")
            .and_then(Value::as_u64)
            .map(|p| p as usize),
    })
}

fn signature(value: &Value, encoding: PositionEncoding) -> Option<Signature> {
    let label = value.get("label")?.as_str()?.to_owned();
    let mut searched = 0;
    let parameters = value
        .get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|parameter| {
            let range = match parameter.get("label")? {
                Value::String(name) => {
                    let at = label[searched..].find(name.as_str())? + searched;
                    searched = at + name.len();
                    at..at + name.len()
                }
                Value::Array(pair) => {
                    let start = pair.first()?.as_u64()? as usize;
                    let end = pair.get(1)?.as_u64()? as usize;
                    encoding.byte_index(&label, start)..encoding.byte_index(&label, end)
                }
                _ => return None,
            };
            Some(ParameterInfo {
                label: range,
                documentation: parameter.get("documentation").and_then(markup),
            })
        })
        .collect();
    Some(Signature {
        documentation: value.get("documentation").and_then(markup),
        parameters,
        active_parameter: value
            .get("activeParameter")
            .and_then(Value::as_u64)
            .map(|p| p as usize),
        label,
    })
}

/// The LSP `SignatureHelpContext` of a request.
pub(crate) fn signature_context(context: &SignatureContext, encoding: PositionEncoding) -> Value {
    let (kind, character) = match context.trigger {
        SignatureTrigger::Invoked => (1, None),
        SignatureTrigger::Character(c) => (2, Some(c.to_string())),
        SignatureTrigger::ContentChange => (3, None),
    };
    let mut lsp = json!({
        "triggerKind": kind,
        "isRetrigger": context.active.is_some(),
    });
    if let Some(character) = character {
        lsp["triggerCharacter"] = character.into();
    }
    if let Some(active) = &context.active {
        let units = |label: &str, byte: usize| encoding.len(&label[..byte.min(label.len())]);
        lsp["activeSignatureHelp"] = json!({
            "signatures": active.signatures.iter().map(|s| json!({
                "label": s.label,
                "parameters": s.parameters.iter().map(|p| json!({
                    "label": [units(&s.label, p.label.start), units(&s.label, p.label.end)],
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "activeSignature": active.active_signature,
            "activeParameter": active.active_parameter,
        });
    }
    lsp
}

/// A definition or references answer (`Location`, `Location[]`,
/// `LocationLink[]` or `null`); files whose text cannot be read are left
/// out.
pub(crate) fn locations(
    value: &Value,
    encoding: PositionEncoding,
    text_of: TextOf<'_>,
) -> Vec<Location> {
    let list: Vec<&Value> = match value {
        Value::Array(list) => list.iter().collect(),
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    };
    let mut locations = Vec::new();
    for location in list {
        let (uri, range_value) = match location.get("targetUri") {
            Some(uri) => (
                uri,
                location
                    .get("targetSelectionRange")
                    .or_else(|| location.get("targetRange")),
            ),
            None => (
                location.get("uri").unwrap_or(&Value::Null),
                location.get("range"),
            ),
        };
        let Some(path) = uri.as_str().and_then(uri::str_to_path) else {
            continue;
        };
        let Some(range_value) = range_value else {
            continue;
        };
        let Ok(text) = text_of(&path) else {
            continue;
        };
        if let Some(range) = range(range_value, &text, encoding) {
            let location = Location { path, range };
            if !locations.contains(&location) {
                locations.push(location);
            }
        }
    }
    locations
}

/// A code action answer (`(Command | CodeAction)[]` or `null`); disabled
/// actions and actions whose edit cannot be converted are left out. The
/// diagnostics an action fixes are the request's own, matched by value.
pub(crate) fn code_actions(
    value: &Value,
    encoding: PositionEncoding,
    server: &ServerId,
    generation: u64,
    diagnostics: &[Diagnostic],
    text_of: TextOf<'_>,
) -> Vec<CodeAction> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|action| {
            code_action(action, encoding, server, generation, diagnostics, text_of)
        })
        .collect()
}

pub(crate) fn code_action(
    value: &Value,
    encoding: PositionEncoding,
    server: &ServerId,
    generation: u64,
    diagnostics: &[Diagnostic],
    text_of: TextOf<'_>,
) -> Option<CodeAction> {
    let title = value.get("title")?.as_str()?.to_owned();
    if value.get("command").is_some_and(Value::is_string) {
        // A bare command.
        let mut action = CodeAction::new(title.clone(), CodeActionKind::Other(String::new()));
        action.command = Some(Command {
            title,
            token: Raw::token(server, generation, value),
        });
        action.server = Some(server.clone());
        return Some(action);
    }
    if value.get("disabled").is_some_and(|d| !d.is_null()) {
        return None;
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map_or(CodeActionKind::Other(String::new()), CodeActionKind::parse);
    let mut action = CodeAction::new(title, kind);
    action.preferred = value.get("isPreferred").and_then(Value::as_bool) == Some(true);
    action.diagnostics = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|echoed| {
            fn sent(d: &Diagnostic) -> Option<&Value> {
                Raw::of(d.token.as_ref()).map(|r| &r.value)
            }
            // The very value, else the same problem as the server
            // normalised it (members dropped or added, defaults left out).
            diagnostics
                .iter()
                .find(|d| sent(d) == Some(echoed))
                .or_else(|| {
                    diagnostics
                        .iter()
                        .find(|d| sent(d).is_some_and(|s| same_diagnostic(s, echoed)))
                })
                .cloned()
        })
        .collect();
    if let Some(edit) = value.get("edit").filter(|e| !e.is_null()) {
        action.edit = Some(workspace_edit(edit, encoding, text_of).ok()?);
    }
    action.command = value
        .get("command")
        .filter(|c| c.is_object())
        .and_then(|command| {
            Some(Command {
                title: command.get("title")?.as_str()?.to_owned(),
                token: Raw::token(server, generation, command),
            })
        });
    action.server = Some(server.clone());
    action.token = Some(Raw::token(server, generation, value));
    Some(action)
}

/// Two LSP diagnostics are the same problem: the same range, and the same
/// code, source and message where both name them.
fn same_diagnostic(sent: &Value, echoed: &Value) -> bool {
    fn ends(diagnostic: &Value) -> Option<[Position; 2]> {
        let range = diagnostic.get("range")?;
        Some([position(range.get("start")?)?, position(range.get("end")?)?])
    }
    fn code(diagnostic: &Value) -> Option<String> {
        match diagnostic.get("code")? {
            Value::String(code) => Some(code.clone()),
            Value::Number(code) => Some(code.to_string()),
            _ => None,
        }
    }
    fn text<'a>(diagnostic: &'a Value, key: &str) -> Option<&'a str> {
        diagnostic.get(key).and_then(Value::as_str)
    }
    fn agree<T: PartialEq>(a: Option<T>, b: Option<T>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }
    ends(sent).is_some()
        && ends(sent) == ends(echoed)
        && agree(code(sent), code(echoed))
        && agree(text(sent, "source"), text(echoed, "source"))
        && agree(text(sent, "message"), text(echoed, "message"))
}

/// A document highlight answer.
pub(crate) fn highlights(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
) -> Vec<DocumentHighlight> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            Some(DocumentHighlight {
                range: range(h.get("range")?, text, encoding)?,
                kind: match h.get("kind").and_then(Value::as_u64) {
                    Some(2) => HighlightKind::Read,
                    Some(3) => HighlightKind::Write,
                    _ => HighlightKind::Text,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use text::Rope;

    use super::*;
    use PositionEncoding::{Utf8, Utf16};
    use code_intel::MarkupKind;

    fn texts(path: &Path) -> Result<Text, String> {
        match path.file_name().and_then(|n| n.to_str()) {
            Some("a.py") => Ok(Text::new(Rope::from_str("import os\nx = \"🦀\"\n"))),
            Some("b.py") => Ok(Text::new(Rope::from_str("pass\n"))),
            _ => Err(format!("cannot read {}", path.display())),
        }
    }

    fn file(name: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\p\{name}"))
        } else {
            PathBuf::from(format!("/p/{name}"))
        }
    }

    fn uri_of(name: &str) -> String {
        uri::from_path(&file(name)).to_string()
    }

    fn r(l1: u32, c1: u32, l2: u32, c2: u32) -> Value {
        json!({"start": {"line": l1, "character": c1}, "end": {"line": l2, "character": c2}})
    }

    #[test]
    fn changes_and_document_changes() {
        let edit = json!({
            "changes": {
                uri_of("a.py"): [
                    {"range": r(1, 5, 1, 7), "newText": "x"},
                    {"range": {"start": {"line": 0}}, "newText": "skipped"},
                ],
            },
        });
        let converted = workspace_edit(&edit, Utf16, &texts).unwrap();
        assert_eq!(converted.files.len(), 1);
        assert_eq!(converted.files[0].path, file("a.py"));
        // UTF-16 columns 5..7 are the crab, one char.
        assert_eq!(
            converted.files[0].edits,
            [TextEdit::new(Point::new(1, 5)..Point::new(1, 6), "x")]
        );

        let edit = json!({
            "documentChanges": [
                {"textDocument": {"uri": uri_of("b.py"), "version": 3}, "edits": [
                    {"range": r(0, 0, 0, 4), "newText": "...", "annotationId": "x"},
                ]},
                {"textDocument": {"uri": uri_of("a.py"), "version": null}, "edits": []},
            ],
            "changes": {uri_of("a.py"): []},
        });
        let converted = workspace_edit(&edit, Utf8, &texts).unwrap();
        assert_eq!(converted.files.len(), 2);
        assert_eq!(converted.files[0].path, file("b.py"));
        assert_eq!(
            converted.files[0].edits,
            [TextEdit::new(Point::new(0, 0)..Point::new(0, 4), "...")]
        );
    }

    #[test]
    fn unsupported_edits_fail() {
        let create = json!({"documentChanges": [{"kind": "create", "uri": uri_of("c.py")}]});
        assert!(
            workspace_edit(&create, Utf16, &texts)
                .unwrap_err()
                .contains("create")
        );
        let unreadable = json!({"changes": {uri_of("missing.py"): []}});
        assert!(workspace_edit(&unreadable, Utf16, &texts).is_err());
        let other = json!({"changes": {"untitled:1": []}});
        assert!(workspace_edit(&other, Utf16, &texts).is_err());
        assert_eq!(
            workspace_edit(&json!(null), Utf16, &texts).unwrap(),
            WorkspaceEdit::default()
        );
    }

    #[test]
    fn diagnostics_as_the_reference_shows_them() {
        let text = Text::new(Rope::from_str("import os\nx = \"🦀\" + y\n"));
        let ruff = ServerId::new("ruff");
        let raw = json!({
            "range": r(0, 7, 0, 9), "severity": 2, "code": "F401", "source": "Ruff",
            "codeDescription": {"href": "https://docs.astral.sh/ruff/rules/unused-import"},
            "message": "`os` imported but unused\n\nhelp: Remove unused import: `os`",
            "tags": [1], "data": {"fix": 1},
        });
        let item = diagnostic(&raw, &text, Utf8, &ruff, 3).unwrap();
        assert_eq!((item.start, item.end), (7, 9));
        let d = &item.diagnostic;
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.summary(), "`os` imported but unused  Ruff(F401)");
        assert!(d.unnecessary && !d.deprecated);
        assert_eq!(d.server, Some(ruff.clone()));
        let token = Raw::of(d.token.as_ref()).unwrap();
        assert_eq!((token.generation, &token.value), (3, &raw));

        // UTF-8 bytes after the crab; numeric codes; severities.
        let ty = json!({"range": r(1, 13, 1, 14), "message": "undefined", "code": 7, "severity": 4, "tags": [2]});
        let item = diagnostic(&ty, &text, Utf8, &ruff, 1).unwrap();
        assert_eq!(item.diagnostic.range, Point::new(1, 10)..Point::new(1, 11));
        assert_eq!(item.diagnostic.code.as_deref(), Some("7"));
        assert_eq!(item.diagnostic.severity, Severity::Information);
        assert!(item.diagnostic.deprecated);
        for (severity, expected) in [
            (json!(1), Severity::Error),
            (json!(3), Severity::WeakWarning),
            (json!(null), Severity::WeakWarning),
        ] {
            let d = json!({"range": r(0, 0, 0, 1), "message": "m", "severity": severity});
            assert_eq!(
                diagnostic(&d, &text, Utf16, &ruff, 1)
                    .unwrap()
                    .diagnostic
                    .severity,
                expected
            );
        }
        assert!(diagnostic(&json!({"message": "no range"}), &text, Utf16, &ruff, 1).is_none());
    }

    #[test]
    fn completion_items() {
        let text = Text::new(Rope::from_str("import os\nos.pa\n"));
        let ty = ServerId::new("ty");
        let list = json!({"isIncomplete": true, "items": [
            {"label": "path", "kind": 9, "detail": "module", "sortText": "0",
             "textEdit": {"range": r(1, 3, 1, 5), "newText": "path"},
             "documentation": {"kind": "markdown", "value": "Paths."}},
            {"label": "pardir", "kind": 21, "labelDetails": {"detail": " (const)", "description": "str"},
             "detail": "ignored as type", "tags": [1],
             "textEdit": {"newText": "pardir", "insert": r(1, 3, 1, 5), "replace": r(1, 3, 1, 5)},
             "insertTextFormat": 2, "preselect": true,
             "additionalTextEdits": [{"range": r(0, 0, 0, 0), "newText": "import x\n"}]},
            {"label": "plain", "insertText": "plain()", "documentation": "text docs"},
            {"no label": true},
        ]});
        let completions = completions(&list, &text, Utf16, &ty, 2);
        assert!(completions.incomplete);
        assert_eq!(completions.items.len(), 3);
        let path = &completions.items[0];
        assert_eq!(path.kind, Some(SymbolKind::Module));
        assert_eq!(path.type_text.as_deref(), Some("module"));
        assert_eq!(path.sort_text(), "0");
        assert_eq!(
            path.insert,
            InsertText::Edit {
                insert: Point::new(1, 3)..Point::new(1, 5),
                replace: Point::new(1, 3)..Point::new(1, 5),
                text: "path".into()
            }
        );
        assert_eq!(path.documentation, Some(Markup::markdown("Paths.")));
        let pardir = &completions.items[1];
        assert_eq!(pardir.tail_text.as_deref(), Some(" (const)"));
        assert_eq!(pardir.type_text.as_deref(), Some("str"));
        assert!(pardir.deprecated && pardir.snippet && pardir.preselect);
        assert_eq!(
            pardir.additional_edits,
            [TextEdit::insert(Point::new(0, 0), "import x\n")]
        );
        let plain = &completions.items[2];
        assert_eq!(plain.insert, InsertText::Text("plain()".into()));
        assert_eq!(plain.documentation, Some(Markup::plain("text docs")));
        assert_eq!(plain.server, Some(ty.clone()));
        assert!(Raw::of(plain.token.as_ref()).is_some());
        // A bare array, and null.
        assert_eq!(completions_len(&json!([{"label": "a"}]), &text), 1);
        assert_eq!(completions_len(&json!(null), &text), 0);
    }

    fn completions_len(value: &Value, text: &Text) -> usize {
        completions(value, text, Utf16, &ServerId::new("x"), 1)
            .items
            .len()
    }

    #[test]
    fn hovers_and_signatures() {
        let text = Text::new(Rope::from_str("print(1)\n"));
        let hover_value = json!({"contents": {"kind": "markdown", "value": "```python\ndef print()\n```"}, "range": r(0, 0, 0, 5)});
        let h = hover(&hover_value, &text, Utf16).unwrap();
        assert_eq!(h.contents.kind, MarkupKind::Markdown);
        assert_eq!(h.range, Some(Point::new(0, 0)..Point::new(0, 5)));
        let old = json!({"contents": ["Doc", {"language": "python", "value": "x: int"}, ""]});
        assert_eq!(
            hover(&old, &text, Utf16).unwrap().contents.text,
            "Doc\n\n```python\nx: int\n```"
        );
        assert!(hover(&json!({"contents": ""}), &text, Utf16).is_none());
        assert!(hover(&json!(null), &text, Utf16).is_none());

        let help = json!({"signatures": [
            {"label": "print(*values: object, sep: str = \" \")", "documentation": "Prints.",
             "parameters": [{"label": "*values: object"}, {"label": [23, 37], "documentation": {"kind": "markdown", "value": "sep"}}]},
            {"label": "f(é, x)", "parameters": [{"label": [2, 3]}, {"label": "x"}], "activeParameter": 1},
        ], "activeSignature": 7, "activeParameter": 0});
        let help = signature_help(&help, Utf16).unwrap();
        assert_eq!(help.active_signature, 1);
        let print = &help.signatures[0];
        assert_eq!(
            &print.label[print.parameters[0].label.clone()],
            "*values: object"
        );
        assert_eq!(
            &print.label[print.parameters[1].label.clone()],
            "sep: str = \" \""
        );
        let f = &help.signatures[1];
        assert_eq!(&f.label[f.parameters[0].label.clone()], "é");
        assert_eq!(&f.label[f.parameters[1].label.clone()], "x");
        assert_eq!(help.active_parameter_of(1), Some(1));
        assert!(signature_help(&json!({"signatures": []}), Utf16).is_none());

        let context = SignatureContext {
            trigger: SignatureTrigger::Character(','),
            active: Some(help.clone()),
        };
        let lsp = signature_context(&context, Utf16);
        assert_eq!(lsp["triggerKind"], 2);
        assert_eq!(lsp["triggerCharacter"], ",");
        assert_eq!(lsp["isRetrigger"], true);
        assert_eq!(
            lsp["activeSignatureHelp"]["signatures"][1]["parameters"][0]["label"],
            json!([2, 3])
        );
        assert_eq!(
            signature_context(&SignatureContext::invoked(), Utf16)["triggerKind"],
            1
        );
    }

    #[test]
    fn locations_of_every_shape() {
        let a = uri_of("a.py");
        let single = json!({"uri": a, "range": r(1, 5, 1, 7)});
        assert_eq!(
            locations(&single, Utf16, &texts),
            [Location {
                path: file("a.py"),
                range: Point::new(1, 5)..Point::new(1, 6)
            }]
        );
        let links = json!([
            {"targetUri": a, "targetRange": r(0, 0, 1, 0), "targetSelectionRange": r(0, 7, 0, 9)},
            {"targetUri": uri_of("missing.py"), "targetSelectionRange": r(0, 0, 0, 1)},
            {"uri": a, "range": r(0, 7, 0, 9)},
        ]);
        assert_eq!(
            locations(&links, Utf16, &texts),
            [Location {
                path: file("a.py"),
                range: Point::new(0, 7)..Point::new(0, 9)
            }]
        );
        assert!(locations(&json!(null), Utf16, &texts).is_empty());
    }

    #[test]
    fn code_actions_and_commands() {
        let ruff = ServerId::new("ruff");
        let raw_diagnostic = json!({"range": r(0, 7, 0, 9), "message": "unused", "code": "F401"});
        let text = Text::new(Rope::from_str("import os\nx = \"🦀\"\n"));
        let diagnostic = diagnostic(&raw_diagnostic, &text, Utf16, &ruff, 1)
            .unwrap()
            .diagnostic;
        let answer = json!([
            {"title": "Ruff (F401): Remove unused import: `os`", "kind": "quickfix", "isPreferred": true,
             "diagnostics": [raw_diagnostic],
             "edit": {"changes": {uri_of("a.py"): [{"range": r(0, 0, 1, 0), "newText": ""}]}}},
            {"title": "Ruff: Organize imports", "kind": "source.organizeImports.ruff", "data": {"uri": "x"}},
            {"title": "Format", "command": "ruff.applyFormat", "arguments": [{"uri": "x"}]},
            {"title": "Disabled", "kind": "quickfix", "disabled": {"reason": "no"}},
            {"title": "Broken edit", "edit": {"changes": {uri_of("missing.py"): []}}},
        ]);
        let actions = code_actions(
            &answer,
            Utf16,
            &ruff,
            1,
            std::slice::from_ref(&diagnostic),
            &texts,
        );
        assert_eq!(actions.len(), 3);
        let fix = &actions[0];
        assert!(fix.kind.is_quick_fix() && fix.preferred);
        assert_eq!(fix.diagnostics, [diagnostic]);
        assert_eq!(
            fix.edit.as_ref().unwrap().files[0].edits[0].range,
            Point::new(0, 0)..Point::new(1, 0)
        );
        let organize = &actions[1];
        assert!(organize.kind.matches(&CodeActionKind::organize_imports()));
        assert!(organize.edit.is_none());
        assert!(Raw::of(organize.token.as_ref()).is_some());
        let format = &actions[2];
        let command = format.command.as_ref().unwrap();
        assert_eq!(command.title, "Format");
        assert_eq!(
            command.token.downcast_ref::<Raw>().unwrap().value["command"],
            "ruff.applyFormat"
        );
    }

    /// Servers send the diagnostics of an action back normalised: members
    /// in another order, `data` or defaults left out, a severity added.
    #[test]
    fn actions_find_their_diagnostics_as_the_server_echoes_them() {
        let ruff = ServerId::new("ruff");
        let text = Text::new(Rope::from_str("import os\nx = 1\n"));
        let sent = json!({"range": r(0, 7, 0, 9), "message": "unused", "code": "F401",
            "source": "Ruff", "data": {"fix": 1}, "tags": [1]});
        let other = json!({"range": r(1, 0, 1, 1), "message": "unused", "code": "F841"});
        let numeric = json!({"range": r(1, 4, 1, 5), "message": "seven", "code": 7});
        let model: Vec<Diagnostic> = [&sent, &other, &numeric]
            .iter()
            .map(|d| diagnostic(d, &text, Utf16, &ruff, 1).unwrap().diagnostic)
            .collect();
        let answer = json!([{"title": "Fix", "kind": "quickfix", "diagnostics": [
            {"severity": 2, "code": "F401", "range": r(0, 7, 0, 9), "message": "unused"},
            {"code": "7", "range": r(1, 4, 1, 5)},
            // Same code and message, another range: not the same problem.
            {"range": r(0, 0, 0, 1), "message": "unused", "code": "F401"},
            {"range": r(0, 7, 0, 9), "message": "something else", "code": "F401"},
        ]}]);
        let actions = code_actions(&answer, Utf16, &ruff, 1, &model, &texts);
        assert_eq!(actions[0].diagnostics, [model[0].clone(), model[2].clone()]);
        // The original token stays, for the next request.
        assert_eq!(
            Raw::of(actions[0].diagnostics[0].token.as_ref())
                .unwrap()
                .value,
            sent
        );
    }

    #[test]
    fn document_highlights() {
        let text = Text::new(Rope::from_str("x = x + 1\n"));
        let found = highlights(
            &json!([{"range": r(0, 0, 0, 1), "kind": 3}, {"range": r(0, 4, 0, 5), "kind": 2}, {"range": r(0, 8, 0, 9)}]),
            &text,
            Utf8,
        );
        assert_eq!(
            found.iter().map(|h| h.kind).collect::<Vec<_>>(),
            [
                HighlightKind::Write,
                HighlightKind::Read,
                HighlightKind::Text
            ]
        );
    }
}
