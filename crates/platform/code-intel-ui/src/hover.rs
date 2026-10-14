//! When the documentation popup shows (`doc` draws it): Quick
//! Documentation at the caret; the popup over the text under the mouse
//! after the reference's tooltip delay of 500 ms
//! (`EditorSettingsExternalizable`), with the problems there, their first
//! quick fix and the symbol's documentation; Error Description at the
//! caret. The popup over the text stays while the mouse is over the text
//! it is about or inside the popup (where it can scroll), and closes a
//! moment after the mouse left both. Escape, typing, a caret move, a blur
//! or another tab close it.

use std::ops::Range;
use std::time::Duration;

use code_intel::{ActionTrigger, CodeActionContext, Hover, Location, Point};
use editor::{Editor, EditorEvent};
use gpui::{App, Context, Entity, EntityId, Modifiers, Task};

use crate::completion::{identifier_end, identifier_start, is_identifier_char};
use crate::doc::{self, DocContent, DocKind, DocLexer, DocPopup, Documentation, Fix};
use crate::popup::{CaretPopup, Placement};
use crate::view::CodeIntelView;

/// How long the mouse rests on the text before the popup shows.
pub const HOVER_DELAY: Duration = Duration::from_millis(500);
/// How long the popup over the text stays after the mouse left the text
/// it is about, so that the mouse can move into it.
pub const HOVER_GRACE: Duration = Duration::from_millis(300);

/// The popup shown: its editor, the range of the text it is about and
/// what it shows.
struct Shown {
    editor: EntityId,
    range: Range<usize>,
    kind: DocKind,
}

/// The state of the documentation popup of a window.
pub(crate) struct DocState {
    pub(crate) popup: Entity<DocPopup>,
    /// The editor and char offset under the mouse (not while the
    /// navigation modifier is held).
    under_mouse: Option<(EntityId, usize)>,
    /// Waits for the hover delay.
    delay: Option<Task<()>>,
    /// The requests of the popup being prepared.
    request: Option<Task<()>>,
    shown: Option<Shown>,
    /// The mouse is inside the popup.
    inside: bool,
    /// Closes the popup over the text after the grace time.
    closing: Option<Task<()>>,
    /// Counts popups asked for; answers for an older one are dropped.
    generation: u64,
}

impl DocState {
    pub(crate) fn new(popup: Entity<DocPopup>) -> Self {
        Self {
            popup,
            under_mouse: None,
            delay: None,
            request: None,
            shown: None,
            inside: false,
            closing: None,
            generation: 0,
        }
    }
}

/// What a request for documentation answers: the documentation and where
/// the symbol is declared.
type DocAnswer = (Option<Hover>, Option<Location>);

impl CodeIntelView {
    /// Closes the documentation popup and forgets the requests for it.
    pub(crate) fn close_doc(&mut self, cx: &mut Context<Self>) {
        let docs = &mut self.docs;
        docs.generation += 1;
        docs.request = None;
        docs.delay = None;
        docs.closing = None;
        docs.inside = false;
        let had = docs.shown.take().is_some();
        if self.popups_mut().hide(doc::CONTEXT) || had {
            cx.notify();
        }
    }

    /// An event of a document's editor.
    pub(crate) fn doc_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Hovered {
                offset, modifiers, ..
            } => self.hovered(editor, *offset, *modifiers, cx),
            EditorEvent::Edited(_) | EditorEvent::SelectionsChanged | EditorEvent::Reloaded => {
                let id = editor.entity_id();
                let ours = self.docs.shown.as_ref().is_some_and(|s| s.editor == id)
                    || self.docs.request.is_some();
                if ours {
                    self.close_doc(cx);
                }
                self.docs.delay = None;
            }
            _ => {}
        }
    }

    /// The mouse moved over an editor's text (`None`: it left the text).
    fn hovered(
        &mut self,
        editor: &Entity<Editor>,
        offset: Option<usize>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let id = editor.entity_id();
        let target = offset.filter(|_| !modifiers.secondary()).map(|o| (id, o));
        if self.docs.under_mouse == target {
            return;
        }
        self.docs.under_mouse = target;
        self.docs.delay = None;
        let over_shown = match (&self.docs.shown, target) {
            (Some(shown), Some((editor, offset))) => {
                shown.kind == DocKind::Hover
                    && shown.editor == editor
                    && shown.range.start <= offset
                    && offset <= shown.range.end
            }
            _ => false,
        };
        if over_shown {
            self.docs.closing = None;
            return;
        }
        if self
            .docs
            .shown
            .as_ref()
            .is_some_and(|s| s.kind == DocKind::Hover)
        {
            self.close_doc_later(cx);
        }
        let Some((_, offset)) = target else {
            return;
        };
        let editor = editor.clone();
        self.docs.delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let _ = this.update(cx, |v, cx| v.hover_due(&editor, offset, cx));
        }));
    }

    /// Closes the popup over the text after the grace time, unless the
    /// mouse is inside it then.
    fn close_doc_later(&mut self, cx: &mut Context<Self>) {
        if self.docs.closing.is_some() {
            return;
        }
        self.docs.closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_GRACE).await;
            let _ = this.update(cx, |v, cx| {
                v.docs.closing = None;
                if !v.docs.inside {
                    v.close_doc(cx);
                }
            });
        }));
    }

    /// The editor of a document lost the focus: its popup closes, unless
    /// the mouse is inside it.
    pub(crate) fn doc_blurred(&mut self, editor: EntityId, cx: &mut Context<Self>) {
        let ours = self.docs.shown.as_ref().is_some_and(|s| s.editor == editor);
        if ours && !self.docs.inside {
            self.close_doc(cx);
        }
    }

    /// The mouse entered or left the documentation popup.
    pub(crate) fn doc_mouse_inside(&mut self, inside: bool, cx: &mut Context<Self>) {
        self.docs.inside = inside;
        if inside {
            self.docs.closing = None;
            self.docs.delay = None;
        } else if self
            .docs
            .shown
            .as_ref()
            .is_some_and(|s| s.kind == DocKind::Hover)
        {
            self.close_doc_later(cx);
        }
    }

    /// The mouse rested on `offset`: the problems there and the
    /// documentation of the symbol, when there is any.
    fn hover_due(&mut self, editor: &Entity<Editor>, offset: usize, cx: &mut Context<Self>) {
        self.docs.delay = None;
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let (doc_id, version) = (document.id.clone(), document.version());
        let buffer = editor.read(cx).buffer();
        let on_identifier = buffer.char_at(offset).is_some_and(is_identifier_char);
        let word = identifier_start(buffer, offset)..identifier_end(buffer, offset);
        let (problems, mark) = doc::problems_at(editor.read(cx).diagnostics(), offset, false);
        if problems.is_empty() && !on_identifier {
            return;
        }
        let Some(services) = self.services().cloned() else {
            return;
        };
        let at = document.versions.point(offset);
        let documentation = on_identifier.then(|| self.request_documentation(&doc_id, at));
        let fix = mark.clone().map(|range| {
            let points = document.versions.point(range.start)..document.versions.point(range.end);
            let context = CodeActionContext {
                only: None,
                diagnostics: self.diagnostics_in(&doc_id, range.clone(), cx),
                trigger: ActionTrigger::Automatic,
            };
            services.code_actions(&doc_id, points, context)
        });
        self.docs.generation += 1;
        let generation = self.docs.generation;
        let editor = editor.clone();
        self.docs.request = Some(cx.spawn(async move |this, cx| {
            let documentation = match documentation {
                Some(request) => request.await,
                None => (None, None),
            };
            let fix = match fix {
                Some(request) => request
                    .await
                    .ok()
                    .and_then(|actions| actions.into_iter().find(|a| a.kind.is_quick_fix())),
                None => None,
            };
            let _ = this.update(cx, |v, cx| {
                if v.docs.generation != generation {
                    return;
                }
                v.docs.request = None;
                let documentation = v.documentation(&editor, version, documentation);
                let mut range = mark.unwrap_or(word);
                if let Some((_, Some(r))) = &documentation {
                    range = range.start.min(r.start)..range.end.max(r.end);
                }
                // The mouse left meanwhile.
                let under = v.docs.under_mouse;
                if !under.is_some_and(|(e, o)| {
                    e == editor.entity_id() && range.start <= o && o <= range.end
                }) {
                    return;
                }
                let content = DocContent {
                    kind: DocKind::Hover,
                    more_actions: !problems.is_empty(),
                    problems,
                    fix: fix.map(|action| Fix { action, version }),
                    documentation: documentation.map(|(d, _)| d),
                };
                v.show_doc(&editor, range, content, cx);
            });
        }));
    }

    /// Asks for the documentation at a position and for where the symbol
    /// is declared (the header of the documentation).
    fn request_documentation(
        &self,
        doc: &code_intel::DocumentId,
        at: Point,
    ) -> impl std::future::Future<Output = DocAnswer> + use<> {
        let services = self.services().cloned();
        let requests = services.map(|s| (s.hover(doc, at), s.definition(doc, at)));
        async move {
            let Some((hover, definition)) = requests else {
                return (None, None);
            };
            let hover = hover
                .await
                .ok()
                .flatten()
                .filter(|h| !h.contents.is_empty());
            if hover.is_none() {
                return (None, None);
            }
            let location = definition.await.ok().and_then(|l| l.into_iter().next());
            (hover, location)
        }
    }

    /// The documentation of an answer, and the range in the current text
    /// it is about when the service said.
    fn documentation(
        &self,
        editor: &Entity<Editor>,
        version: u64,
        (hover, location): DocAnswer,
    ) -> Option<(Documentation, Option<Range<usize>>)> {
        let hover = hover?;
        let document = self.document_of(editor)?;
        let provider = self.hook().0.as_ref();
        let location = location.and_then(|l| provider.doc_location(&l.path));
        let lexer = DocLexer {
            provider,
            language: &document.language,
        };
        let documentation = Documentation::new(&hover.contents, location, Some(&lexer))?;
        let range = hover
            .range
            .and_then(|r| document.versions.range(version, r));
        Some((documentation, range))
    }

    /// Shows the popup about a range of an editor's text, under its line.
    fn show_doc(
        &mut self,
        editor: &Entity<Editor>,
        range: Range<usize>,
        content: DocContent,
        cx: &mut Context<Self>,
    ) {
        if content.is_empty() {
            self.close_doc(cx);
            return;
        }
        let kind = content.kind;
        let height = content.estimated_height(cx);
        self.docs.popup.update(cx, |p, cx| p.set(content, cx));
        self.docs.shown = Some(Shown {
            editor: editor.entity_id(),
            range: range.clone(),
            kind,
        });
        self.docs.closing = None;
        let popup = self.docs.popup.clone();
        self.popups_mut().show(CaretPopup {
            context: doc::CONTEXT,
            editor: editor.clone(),
            anchor: range.start,
            placement: Placement::Below,
            inset: gpui::px(0.0),
            height: Some(height),
            content: popup.into(),
        });
        cx.notify();
    }

    /// Quick Documentation at the caret of an editor: the documentation of
    /// the symbol at the caret (or right before it).
    pub(crate) fn quick_doc_at_caret(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let buffer = editor.read(cx).buffer();
        let offset = symbol_offset(buffer, query.offset);
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let at = document.versions.point(offset);
        let request = self.request_documentation(&query.doc, at);
        self.docs.generation += 1;
        let generation = self.docs.generation;
        let (editor, version) = (editor.clone(), query.version);
        self.docs.request = Some(cx.spawn(async move |this, cx| {
            let answer = request.await;
            let _ = this.update(cx, |v, cx| {
                if v.docs.generation != generation {
                    return;
                }
                v.docs.request = None;
                let Some((documentation, _)) = v.documentation(&editor, version, answer) else {
                    return;
                };
                let caret = editor.read(cx).selections().primary().head;
                let content = DocContent {
                    kind: DocKind::QuickDoc,
                    problems: Vec::new(),
                    fix: None,
                    more_actions: false,
                    documentation: Some(documentation),
                };
                v.show_doc(&editor, caret..caret, content, cx);
            });
        }));
    }

    /// Error Description: the problems at the caret of an editor.
    pub(crate) fn error_description_at_caret(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        if self.document_of(editor).is_none() {
            return;
        }
        let caret = editor.read(cx).selections().primary().head;
        let (problems, _) = doc::problems_at(editor.read(cx).diagnostics(), caret, true);
        if problems.is_empty() {
            return;
        }
        self.close_doc(cx);
        let content = DocContent {
            kind: DocKind::Problems,
            problems,
            fix: None,
            more_actions: false,
            documentation: None,
        };
        self.show_doc(editor, caret..caret, content, cx);
    }

    /// Applies the quick fix of the popup's actions row.
    pub(crate) fn apply_doc_fix(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(editor) = self
            .docs
            .shown
            .as_ref()
            .map(|s| s.editor)
            .and_then(|id| self.editor_by_id(id))
        else {
            return;
        };
        let fix = self
            .docs
            .popup
            .read(cx)
            .content()
            .and_then(|c| c.fix.clone());
        self.close_doc(cx);
        if let Some(fix) = fix {
            self.apply_code_action(&editor, fix.version, fix.action, cx);
        }
    }

    /// The documentation popup's state for `code.state`.
    pub(crate) fn doc_state_json(&self, cx: &App) -> serde_json::Value {
        if !self.popups().is_shown(doc::CONTEXT) {
            return serde_json::Value::Null;
        }
        self.docs
            .popup
            .read(cx)
            .content()
            .map(DocContent::to_json)
            .unwrap_or_default()
    }

    /// The editor of a document by its entity id.
    pub(crate) fn editor_by_id(&self, id: EntityId) -> Option<Entity<Editor>> {
        self.documents()
            .find(|d| d.editor.entity_id() == id)
            .map(|d| d.editor.clone())
    }
}

/// Where documentation is asked for at a caret: at the identifier char
/// under it, or the one right before it (the caret at the end of a name).
pub fn symbol_offset(buffer: &text::Buffer, caret: usize) -> usize {
    let at = buffer.char_at(caret).is_some_and(is_identifier_char);
    let before = caret > 0 && buffer.char_at(caret - 1).is_some_and(is_identifier_char);
    if !at && before { caret - 1 } else { caret }
}

#[cfg(test)]
mod tests {
    use text::Buffer;

    use super::*;

    #[test]
    fn documentation_is_asked_for_at_the_name_before_the_caret() {
        let buffer = Buffer::new("json.dumps(x)");
        assert_eq!(symbol_offset(&buffer, 5), 5);
        assert_eq!(symbol_offset(&buffer, 10), 9);
        assert_eq!(symbol_offset(&buffer, 4), 3);
        assert_eq!(symbol_offset(&buffer, 0), 0);
    }
}
