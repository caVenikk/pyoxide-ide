//! Highlight Usages in File: the other occurrences of the symbol at the
//! caret are drawn with the background of the theme's read and write
//! identifiers (the reference's `IdentifierHighlighterPass`). The services
//! answer `document_highlights`; the highlights stay while the caret moves
//! inside them, go when it leaves them or the text changes, and come again
//! for the next symbol after a short pause.

use std::time::Duration;

use code_intel::{DocumentHighlight, HighlightKind};
use editor::{Editor, EditorEvent, UsageHighlight};
use gpui::{Context, Entity, Task};

use crate::hover::symbol_offset;
use crate::view::CodeIntelView;

/// The pause after the caret moved before the services are asked.
const DELAY: Duration = Duration::from_millis(150);

/// The request for the symbol at the caret that is waiting or in flight.
#[derive(Default)]
pub(crate) struct OccurrenceState {
    request: Option<Task<()>>,
}

impl CodeIntelView {
    /// Events of a document's editor: a moved caret asks for the
    /// occurrences of the symbol under it.
    pub(crate) fn occurrence_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::SelectionsChanged => self.caret_moved_for_occurrences(editor, cx),
            EditorEvent::Edited(_) | EditorEvent::Reloaded => {
                // The editor dropped the highlights with the edit.
                self.occurrences.request = None;
            }
            _ => {}
        }
    }

    fn caret_moved_for_occurrences(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        self.occurrences.request = None;
        let Some(query) = self.caret_query(editor, cx) else {
            self.clear_occurrences(editor, cx);
            return;
        };
        let (empty, kept) = {
            let e = editor.read(cx);
            let empty = e.selections().primary().is_empty();
            let kept = e
                .usage_highlights()
                .iter()
                .any(|u| u.range.start <= query.offset && query.offset <= u.range.end);
            (empty, kept)
        };
        if !empty {
            self.clear_occurrences(editor, cx);
            return;
        }
        // The caret is still on a symbol that is highlighted.
        if kept {
            return;
        }
        self.clear_occurrences(editor, cx);
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let offset = symbol_offset(editor.read(cx).buffer(), query.offset);
        let doc = document.id.clone();
        let at = document.versions.point(offset);
        let version = document.version();
        let editor = editor.clone();
        self.occurrences.request = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELAY).await;
            let answer = services.document_highlights(&doc, at).await;
            let _ = this.update(cx, |v, cx| {
                v.occurrences.request = None;
                if let Ok(items) = answer {
                    v.show_occurrences(&editor, version, items, cx);
                }
            });
        }));
    }

    fn show_occurrences(
        &mut self,
        editor: &Entity<Editor>,
        version: u64,
        items: Vec<DocumentHighlight>,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let mut found: Vec<UsageHighlight> = items
            .into_iter()
            .filter_map(|item| {
                document
                    .versions
                    .range(version, item.range)
                    .map(|range| UsageHighlight {
                        range,
                        write: item.kind == HighlightKind::Write,
                    })
            })
            .collect();
        found.sort_by_key(|u| (u.range.start, u.range.end));
        editor.update(cx, |e, cx| e.set_usage_highlights(found, cx));
    }

    fn clear_occurrences(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        if !editor.read(cx).usage_highlights().is_empty() {
            editor.update(cx, |e, cx| e.set_usage_highlights(Vec::new(), cx));
        }
    }
}
