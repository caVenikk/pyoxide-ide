//! The diagnostics of the open documents, by server.
//!
//! Each server's latest result for a document is kept with the document
//! version its positions refer to (char offsets in that text). Whenever a
//! result arrives, every server's items of the document are moved through
//! the edits made since their version and sent as one
//! [`IntelEvent::Diagnostics`] in the current text; items whose text is no
//! longer known (too old, or the document was reloaded) are dropped. The
//! UI merges items with equal ranges into one mark (the reference counts
//! each problem).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use code_intel::{Diagnostic, DocumentId, EventHub, IntelEvent, ServerId};

use crate::documents::{Bias, Documents};
use crate::encoding::Text;

/// A diagnostic with its range as char offsets in the text it was
/// computed for.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub start: usize,
    pub end: usize,
    pub diagnostic: Diagnostic,
}

#[derive(Debug)]
struct Entry {
    server: ServerId,
    version: u64,
    items: Vec<Item>,
}

#[derive(Debug, Default)]
pub(crate) struct Diagnostics {
    by_doc: Mutex<HashMap<DocumentId, Vec<Entry>>>,
}

impl Diagnostics {
    fn lock(&self) -> MutexGuard<'_, HashMap<DocumentId, Vec<Entry>>> {
        self.by_doc.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A server's diagnostics of a document, computed for `version`, replace
    /// its earlier ones.
    pub fn set(
        &self,
        documents: &Documents,
        hub: &EventHub,
        doc: &DocumentId,
        server: &ServerId,
        version: u64,
        items: Vec<Item>,
    ) {
        if !documents.is_open(doc) {
            return;
        }
        let mut by_doc = self.lock();
        let entries = by_doc.entry(doc.clone()).or_default();
        entries.retain(|e| e.server != *server);
        entries.push(Entry {
            server: server.clone(),
            version,
            items,
        });
        emit(documents, hub, doc, entries);
    }

    /// The server stopped: its diagnostics go.
    pub fn clear_server(&self, documents: &Documents, hub: &EventHub, server: &ServerId) {
        let mut by_doc = self.lock();
        for (doc, entries) in by_doc.iter_mut() {
            let before = entries.len();
            entries.retain(|e| e.server != *server);
            if entries.len() != before {
                emit(documents, hub, doc, entries);
            }
        }
    }

    /// The document was closed.
    pub fn close(&self, doc: &DocumentId) {
        self.lock().remove(doc);
    }
}

/// Sends every server's items of a document in its current text.
fn emit(documents: &Documents, hub: &EventHub, doc: &DocumentId, entries: &mut Vec<Entry>) {
    let Some(current) = documents.get(doc) else {
        return;
    };
    let text = Text::new(current.text);
    let mut items = Vec::new();
    entries.retain_mut(|entry| {
        let Some((version, mapper)) = documents.mapper(doc, entry.version) else {
            // Its text is gone: stale.
            return false;
        };
        for item in &mut entry.items {
            item.start = mapper.map(item.start, Bias::Start);
            item.end = mapper.map(item.end, Bias::End).max(item.start);
        }
        entry.version = version;
        items.extend(entry.items.iter().map(|item| {
            let mut diagnostic = item.diagnostic.clone();
            diagnostic.range = text.offset_to_point(item.start)..text.offset_to_point(item.end);
            diagnostic
        }));
        true
    });
    hub.emit(IntelEvent::Diagnostics {
        doc: doc.clone(),
        version: current.version,
        items,
    });
}

#[cfg(test)]
mod tests {
    use code_intel::{Point, Severity};
    use text::{Buffer, CaretPlacement, Edit, EditKind, Rope, SelectionSet};

    use super::*;

    fn item(start: usize, end: usize, message: &str, server: &str) -> Item {
        let mut diagnostic = Diagnostic::new(
            Point::default()..Point::default(),
            Severity::Warning,
            message,
        );
        diagnostic.server = Some(ServerId::new(server));
        Item {
            start,
            end,
            diagnostic,
        }
    }

    fn next(rx: &flume::Receiver<IntelEvent>) -> (u64, Vec<(String, std::ops::Range<Point>)>) {
        match rx.try_recv().unwrap() {
            IntelEvent::Diagnostics { version, items, .. } => (
                version,
                items.into_iter().map(|d| (d.message, d.range)).collect(),
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_servers_items_in_the_current_text() {
        let documents = Documents::default();
        let hub = EventHub::new();
        let rx = hub.subscribe();
        let store = Diagnostics::default();
        let doc = DocumentId::new("/p/a.py");
        let ruff = ServerId::new("ruff");
        let ty = ServerId::new("ty");
        let mut buffer = Buffer::new("import os\nfoo()\n");
        documents.open(&doc, "python", 1, buffer.rope());

        store.set(
            &documents,
            &hub,
            &doc,
            &ruff,
            1,
            vec![item(7, 9, "unused", "ruff")],
        );
        assert_eq!(
            next(&rx),
            (
                1,
                vec![("unused".into(), Point::new(0, 7)..Point::new(0, 9))]
            )
        );

        // An edit before both problems: ty's answer for version 1 comes
        // after it and is moved, ruff's stay where the UI moved them.
        let applied = buffer.transact(
            &mut SelectionSet::caret(0),
            vec![Edit::insert(0, "# x\n")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        documents.change(&doc, 2, Some(&applied), buffer.rope());
        store.set(
            &documents,
            &hub,
            &doc,
            &ty,
            1,
            vec![item(10, 13, "undefined", "ty")],
        );
        assert_eq!(
            next(&rx),
            (
                2,
                vec![
                    ("unused".into(), Point::new(1, 7)..Point::new(1, 9)),
                    ("undefined".into(), Point::new(2, 0)..Point::new(2, 3)),
                ]
            )
        );

        // ruff's new answer replaces its old one; ty's stays.
        store.set(&documents, &hub, &doc, &ruff, 2, Vec::new());
        let (_, items) = next(&rx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "undefined");

        // A reload drops answers for older texts.
        documents.change(&doc, 3, None, &Rope::from_str("x\n"));
        store.set(
            &documents,
            &hub,
            &doc,
            &ruff,
            3,
            vec![item(0, 1, "new", "ruff")],
        );
        assert_eq!(
            next(&rx),
            (3, vec![("new".into(), Point::new(0, 0)..Point::new(0, 1))])
        );

        store.clear_server(&documents, &hub, &ruff);
        assert_eq!(next(&rx), (3, vec![]));
        store.clear_server(&documents, &hub, &ty);
        assert!(rx.try_recv().is_err());

        store.close(&doc);
        documents.close(&doc);
        store.set(
            &documents,
            &hub,
            &doc,
            &ruff,
            3,
            vec![item(0, 1, "late", "ruff")],
        );
        assert!(rx.try_recv().is_err());
    }
}
