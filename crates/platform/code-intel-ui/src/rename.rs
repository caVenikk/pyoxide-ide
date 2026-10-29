//! Rename (Shift+F6): the symbol at the caret and its usages get a new
//! name. The services are asked first whether the symbol can be renamed
//! (they refuse what they cannot, with a message that shows as a hint at
//! the caret); the workbench then shows the Rename dialog
//! ([`CodeIntelEvent::RenameDialog`]) and reports the name typed
//! ([`CodeIntelView::rename_confirmed`]), which asks the services for the
//! edits and applies them as one undo step per document.

use code_intel::{DocumentId, IntelError, Point};
use editor::Editor;
use gpui::{Context, Entity, Task};

use crate::completion::{identifier_end, identifier_start};
use crate::hover::symbol_offset;
use crate::view::{CodeIntelEvent, CodeIntelView};

/// The hint when the caret is not on a name.
pub const NO_SYMBOL: &str =
    "Cannot perform refactoring.\nThe caret should be positioned at the symbol to be refactored.";
/// The hint when the services cannot rename.
pub const NOT_SUPPORTED: &str = "Cannot perform refactoring.\nThe language services do not rename.";

/// The rename the dialog is open for.
struct Pending {
    doc: DocumentId,
    at: Point,
    version: u64,
    /// The name as it was in the text, for "unchanged".
    name: String,
}

#[derive(Default)]
pub(crate) struct RenameState {
    request: Option<Task<()>>,
    pending: Option<Pending>,
}

/// The name inside the quotes of a quoted identifier (`"a b"`, `` `a` ``,
/// `[a]`), else the text as it is.
pub fn unquoted(text: &str) -> &str {
    let mut chars = text.chars();
    let (Some(first), Some(last)) = (chars.next(), chars.next_back()) else {
        return text;
    };
    let quoted = matches!((first, last), ('"', '"') | ('`', '`') | ('[', ']'));
    if quoted && text.len() > 2 {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

impl CodeIntelView {
    /// Rename at the caret of an editor.
    pub(crate) fn rename_at_caret(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let offset = symbol_offset(editor.read(cx).buffer(), query.offset);
        let doc = document.id.clone();
        let at = document.versions.point(offset);
        let version = document.version();
        let highlights = services.document_highlights(&doc, at);
        let editor = editor.clone();
        self.rename.pending = None;
        self.rename.request = Some(cx.spawn(async move |this, cx| {
            // The exact range of the name (a quoted one has spaces), else
            // the identifier under the caret.
            let range = highlights.await.ok().and_then(|items| {
                items
                    .into_iter()
                    .map(|h| h.range)
                    .find(|r| r.start <= at && at <= r.end)
            });
            let name = this
                .update(cx, |v, cx| {
                    let document = v.document_of(&editor)?;
                    let buffer = editor.read(cx).buffer();
                    let range = match &range {
                        Some(range) => document.versions.range(version, range.clone())?,
                        None => identifier_start(buffer, offset)..identifier_end(buffer, offset),
                    };
                    let text = buffer.slice(range.start..range.end);
                    Some(text).filter(|t| !t.is_empty())
                })
                .ok()
                .flatten();
            let Some(name) = name else {
                let _ = this.update(cx, |v, cx| {
                    v.rename.request = None;
                    v.show_hint(&editor, offset, NO_SYMBOL, cx);
                });
                return;
            };
            let shown = unquoted(&name).to_owned();
            // Can it be renamed? The services answer with the edits of a
            // rename to the name it has, or refuse.
            let probe = services.rename(&doc, at, shown.clone()).await;
            let _ = this.update(cx, |v, cx| {
                v.rename.request = None;
                match probe {
                    Ok(_) => {
                        v.rename.pending = Some(Pending {
                            doc,
                            at,
                            version,
                            name: shown.clone(),
                        });
                        cx.emit(CodeIntelEvent::RenameDialog { name: shown });
                    }
                    Err(IntelError::Other(message)) => {
                        v.show_hint(&editor, offset, &format!("Cannot rename: {message}"), cx)
                    }
                    Err(_) => v.show_hint(&editor, offset, NOT_SUPPORTED, cx),
                }
            });
        }));
    }

    /// The Rename dialog was confirmed with `new_name`: the services make
    /// the edits, which are applied; the dialog hears the result
    /// ([`CodeIntelEvent::RenameDone`], [`CodeIntelEvent::RenameRejected`]).
    pub fn rename_confirmed(&mut self, new_name: String, cx: &mut Context<Self>) {
        let (Some(pending), Some(services)) =
            (self.rename.pending.as_ref(), self.services().cloned())
        else {
            return;
        };
        let new_name = new_name.trim().to_owned();
        if new_name == pending.name {
            self.rename.pending = None;
            cx.emit(CodeIntelEvent::RenameDone);
            return;
        }
        let request = services.rename(&pending.doc, pending.at, new_name);
        let version = pending.version;
        let path = pending.doc.path().to_owned();
        self.rename.request = Some(cx.spawn(async move |this, cx| {
            let answer = request.await;
            let _ = this.update(cx, |v, cx| {
                v.rename.request = None;
                match answer {
                    Ok(edit) => {
                        v.rename.pending = None;
                        let complete = v.apply_workspace_edit(&edit, Some((&path, version)), cx);
                        if !complete {
                            cx.emit(CodeIntelEvent::Notice(
                                "Some of the usages could not be renamed".into(),
                            ));
                        }
                        cx.emit(CodeIntelEvent::RenameDone);
                    }
                    Err(IntelError::Other(message)) => {
                        cx.emit(CodeIntelEvent::RenameRejected(message))
                    }
                    Err(_) => cx.emit(CodeIntelEvent::RenameRejected(NOT_SUPPORTED.into())),
                }
            });
        }));
    }

    /// The Rename dialog was closed without a rename.
    pub fn rename_cancelled(&mut self, cx: &mut Context<Self>) {
        self.rename.pending = None;
        self.rename.request = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_are_not_part_of_the_name() {
        assert_eq!(unquoted("\"My Alias\""), "My Alias");
        assert_eq!(unquoted("`t`"), "t");
        assert_eq!(unquoted("[t]"), "t");
        assert_eq!(unquoted("abc"), "abc");
        assert_eq!(unquoted("\""), "\"");
        assert_eq!(unquoted("\"\""), "\"\"");
    }
}
