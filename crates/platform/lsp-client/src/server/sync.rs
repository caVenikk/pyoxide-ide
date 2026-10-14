//! The documents of a server and their diagnostics, as the reference's
//! `LspDocumentSyncManager`, `LspWatchedFiles` and highlighting caches:
//!
//! - `didOpen` when a document of the server's languages opens or the
//!   server starts (with the open documents), `didClose` when it closes;
//! - every change at once as one `didChange` with an incremental change
//!   per applied edit, or the whole text for servers that sync fully (and
//!   after a reload); a version counter per document and server, from 1;
//! - `didSave` only when the server asked for it;
//! - `workspace/didChangeWatchedFiles` for the application's changes that
//!   match the server's registered watchers;
//! - diagnostics pushed by the server, and pulled by the client after
//!   opening, 300 ms after the last edit (the reference's highlighting
//!   delay), and when the server asks to refresh them.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use code_intel::{AppliedEdit, DocumentId, Feature, FileChange, FileChangeKind, Rope};
use serde_json::{Value, json};

use super::{LanguageServer, Session};
use crate::capabilities::SyncKind;
use crate::convert;
use crate::diagnostics::Item;
use crate::encoding::{PositionEncoding, Text};
use crate::glob::Watcher;
use crate::rpc::Params;
use crate::uri;

/// How long after the last edit a document's diagnostics are pulled.
pub(crate) const PULL_DELAY: Duration = Duration::from_millis(300);

/// LSP versions remembered per document, to place pushed diagnostics.
const VERSIONS: usize = 64;

/// A document open on a server.
#[derive(Debug)]
pub(crate) struct Synced {
    pub uri: String,
    /// The last version sent.
    pub lsp_version: i32,
    /// The document version it is.
    pub doc_version: u64,
    /// Recent (LSP version, document version) pairs.
    versions: VecDeque<(i32, u64)>,
}

impl Synced {
    fn record(&mut self) {
        self.versions
            .push_back((self.lsp_version, self.doc_version));
        if self.versions.len() > VERSIONS {
            self.versions.pop_front();
        }
    }

    fn doc_version_of(&self, lsp_version: i32) -> Option<u64> {
        self.versions
            .iter()
            .find(|(lsp, _)| *lsp == lsp_version)
            .map(|(_, doc)| *doc)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    /// The URI of a document open on the server.
    pub fn uri_of(&self, doc: &DocumentId) -> Option<String> {
        lock(&self.docs).get(doc).map(|s| s.uri.clone())
    }

    /// The server offers a feature: in its `initialize` answer or by a
    /// registration.
    pub fn provides(&self, capability: &str, method: &str) -> bool {
        self.info().is_some_and(|info| info.provides(capability))
            || self
                .state
                .registrations()
                .iter()
                .any(|r| r.method == method)
    }

    /// A registration's options, or a static capability's.
    pub fn options(&self, capability: &str, method: &str) -> Option<Value> {
        let registered = self
            .state
            .registrations()
            .iter()
            .rev()
            .find(|r| r.method == method)
            .map(|r| r.options.clone());
        registered.or_else(|| {
            self.info()
                .and_then(|info| info.capabilities.get(capability).cloned())
        })
    }
}

impl LanguageServer {
    /// Opens the open documents of its languages on a session that just
    /// started.
    pub(super) fn open_all(&self, session: &Arc<Session>) {
        let spec = self.spec();
        for (doc, snapshot) in self.cx.documents.all() {
            if spec.serves(&snapshot.language) {
                self.open_on(session, &doc);
            }
        }
        self.update_documents();
    }

    /// A document was opened.
    pub fn open(&self, doc: &DocumentId) {
        if let Some(session) = self.running_session() {
            self.open_on(&session, doc);
            self.update_documents();
        }
    }

    fn open_on(&self, session: &Arc<Session>, doc: &DocumentId) {
        {
            let mut docs = lock(&session.docs);
            if docs.contains_key(doc) {
                return;
            }
            let Some(snapshot) = self.cx.documents.get(doc) else {
                return;
            };
            let uri = uri::from_path(doc.path()).to_string();
            if session.info().is_some_and(|info| info.open_close()) {
                let params = {
                    let uri = uri.clone();
                    move || {
                        json!({"textDocument": {
                            "uri": uri,
                            "languageId": snapshot.language,
                            "version": 1,
                            "text": snapshot.text.to_string(),
                        }})
                    }
                };
                session
                    .connection
                    .notify("textDocument/didOpen", Params::Lazy(Box::new(params)));
            }
            let mut synced = Synced {
                uri,
                lsp_version: 1,
                doc_version: snapshot.version,
                versions: VecDeque::new(),
            };
            synced.record();
            docs.insert(doc.clone(), synced);
        }
        // The client pulls a just opened document's diagnostics.
        self.schedule_pull(session, doc, Duration::ZERO);
    }

    /// A document changed from version `base` (`None`: replaced as a whole)
    /// to `version` with `edits`.
    pub fn change(
        &self,
        doc: &DocumentId,
        base: Option<u64>,
        version: u64,
        edits: &[AppliedEdit],
        text: &Rope,
    ) {
        let Some(session) = self.running_session() else {
            return;
        };
        let Some(info) = session.info() else {
            return;
        };
        let encoding = info.encoding;
        let sync = info.sync_kind();
        {
            let mut docs = lock(&session.docs);
            let Some(synced) = docs.get_mut(doc) else {
                return;
            };
            if synced.doc_version == version {
                // Opened with this text already.
                return;
            }
            if sync != SyncKind::None {
                synced.lsp_version += 1;
                let incremental = sync == SyncKind::Incremental
                    && base == Some(synced.doc_version)
                    && encoding != PositionEncoding::Utf32;
                let uri = synced.uri.clone();
                let lsp_version = synced.lsp_version;
                let changes: Box<dyn FnOnce() -> Value + Send> = if incremental {
                    let edits = edits.to_vec();
                    Box::new(move || {
                        Value::Array(
                            edits
                                .iter()
                                .filter_map(|edit| {
                                    let range = encoding.applied_range(edit)?;
                                    Some(json!({"range": range, "text": &*edit.text}))
                                })
                                .collect(),
                        )
                    })
                } else {
                    let text = text.clone();
                    Box::new(move || json!([{ "text": text.to_string() }]))
                };
                session.connection.notify(
                    "textDocument/didChange",
                    Params::Lazy(Box::new(move || {
                        json!({
                            "textDocument": {"uri": uri, "version": lsp_version},
                            "contentChanges": changes(),
                        })
                    })),
                );
            }
            synced.doc_version = version;
            synced.record();
        }
        self.schedule_pull(&session, doc, super::PULL_DELAY);
    }

    /// A document was written to disk.
    pub fn save(&self, doc: &DocumentId, text: &Rope) {
        let Some(session) = self.running_session() else {
            return;
        };
        let Some(include_text) = session.info().and_then(|info| info.save()) else {
            return;
        };
        let Some(uri) = session.uri_of(doc) else {
            return;
        };
        let text = text.clone();
        session.connection.notify(
            "textDocument/didSave",
            Params::Lazy(Box::new(move || {
                let mut params = json!({"textDocument": {"uri": uri}});
                if include_text {
                    params["text"] = text.to_string().into();
                }
                params
            })),
        );
    }

    /// A document was closed.
    pub fn close(&self, doc: &DocumentId) {
        let session = self.slot().session.clone();
        let Some(session) = session else {
            return;
        };
        self.cx.pulls.cancel(&(self.id.clone(), doc.clone()));
        lock(&session.pulls).remove(doc);
        lock(&session.result_ids).remove(doc);
        let removed = lock(&session.docs).remove(doc);
        if let Some(synced) = removed {
            if session.info().is_some_and(|info| info.open_close()) {
                session.connection.notify(
                    "textDocument/didClose",
                    json!({"textDocument": {"uri": synced.uri}}),
                );
            }
            self.update_documents();
        }
    }

    /// Files the application changed on disk, for the server's watchers.
    pub fn files_changed(&self, changes: &[FileChange]) {
        let Some(session) = self.running_session() else {
            return;
        };
        let watchers: Vec<Watcher> = session
            .state
            .registrations()
            .iter()
            .filter(|r| r.method == "workspace/didChangeWatchedFiles")
            .flat_map(|r| Watcher::from_options(&r.options))
            .collect();
        let matching: Vec<Value> = changes
            .iter()
            .filter(|change| {
                watchers
                    .iter()
                    .any(|w| w.matches(&change.path, change.kind))
            })
            .map(|change| {
                json!({
                    "uri": uri::from_path(&change.path).as_str(),
                    "type": match change.kind {
                        FileChangeKind::Created => 1,
                        FileChangeKind::Changed => 2,
                        FileChangeKind::Deleted => 3,
                    },
                })
            })
            .collect();
        if !matching.is_empty() {
            session.connection.notify(
                "workspace/didChangeWatchedFiles",
                json!({ "changes": matching }),
            );
        }
    }

    /// The documents in the status follow the session's.
    fn update_documents(&self) {
        let mut slot = self.slot();
        let mut documents: Vec<DocumentId> = match &slot.session {
            Some(session) => lock(&session.docs).keys().cloned().collect(),
            None => Vec::new(),
        };
        documents.sort();
        if slot.status.documents != documents {
            slot.status.documents = documents;
            self.emit(&slot);
        }
    }

    // ---- diagnostics -------------------------------------------------------

    /// The server's diagnostics are wanted (its spec lists them).
    fn wants_diagnostics(&self) -> bool {
        self.spec().features.contains(Feature::Diagnostics)
    }

    /// The server answers `textDocument/diagnostic`.
    fn pulls(&self, session: &Session) -> bool {
        self.wants_diagnostics()
            && session.provides("diagnosticProvider", "textDocument/diagnostic")
    }

    fn schedule_pull(&self, session: &Session, doc: &DocumentId, delay: Duration) {
        if self.pulls(session) {
            self.cx
                .pulls
                .schedule((self.id.clone(), doc.clone()), self.me.clone(), delay);
        }
    }

    /// Pulls a document's diagnostics (on the debouncer's thread).
    pub(crate) fn pull(&self, doc: &DocumentId) {
        let Some(session) = self.running_session() else {
            return;
        };
        if !self.pulls(&session) {
            return;
        }
        let Some((uri, version)) = lock(&session.docs)
            .get(doc)
            .map(|s| (s.uri.clone(), s.doc_version))
        else {
            return;
        };
        let mut params = json!({"textDocument": {"uri": uri}});
        let identifier = session
            .options("diagnosticProvider", "textDocument/diagnostic")
            .and_then(|o| o.get("identifier").cloned())
            .filter(Value::is_string);
        if let Some(identifier) = identifier {
            params["identifier"] = identifier;
        }
        if let Some(previous) = lock(&session.result_ids).get(doc) {
            params["previousResultId"] = previous.clone().into();
        }
        let server = self.me.clone();
        let weak_session = Arc::downgrade(&session);
        let pulled = doc.clone();
        let request =
            session
                .connection
                .request("textDocument/diagnostic", params, move |report| {
                    if let (Some(server), Some(session)) =
                        (server.upgrade(), weak_session.upgrade())
                    {
                        server.pulled(&session, &pulled, version, &report);
                    }
                    Ok(())
                });
        // A pull still running for an older text is cancelled.
        lock(&session.pulls).insert(doc.clone(), request);
    }

    fn pulled(&self, session: &Arc<Session>, doc: &DocumentId, version: u64, report: &Value) {
        if let Some(id) = report.get("resultId").and_then(Value::as_str) {
            lock(&session.result_ids).insert(doc.clone(), id.to_owned());
        }
        if report.get("kind").and_then(Value::as_str) != Some("full") {
            // "unchanged": the last items stand.
            return;
        }
        let items = report.get("items").and_then(Value::as_array);
        self.set_diagnostics(
            session,
            doc,
            version,
            items.map(Vec::as_slice).unwrap_or_default(),
        );
    }

    /// `textDocument/publishDiagnostics`.
    pub(super) fn published_diagnostics(&self, generation: u64, params: Value) {
        if !self.wants_diagnostics() {
            return;
        }
        let Some(session) = self.session_of(generation) else {
            return;
        };
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let path = uri::str_to_path(uri);
        let found = lock(&session.docs).iter().find_map(|(doc, synced)| {
            let same = synced.uri == uri || path.as_deref() == Some(doc.path());
            if !same {
                return None;
            }
            // Without a version: the last text sent.
            let version = match params.get("version").and_then(Value::as_i64) {
                Some(v) => synced.doc_version_of(i32::try_from(v).ok()?)?,
                None => synced.doc_version,
            };
            Some((doc.clone(), version))
        });
        let Some((doc, version)) = found else {
            // Not open, or for a text this client no longer knows.
            return;
        };
        let items = params.get("diagnostics").and_then(Value::as_array);
        self.set_diagnostics(
            &session,
            &doc,
            version,
            items.map(Vec::as_slice).unwrap_or_default(),
        );
    }

    /// A server's diagnostics of a document in the text of `version`.
    fn set_diagnostics(&self, session: &Session, doc: &DocumentId, version: u64, items: &[Value]) {
        let Some(rope) = self.cx.documents.text_at(doc, version) else {
            return;
        };
        let text = Text::new(rope);
        let encoding = session.encoding();
        let spec = self.spec();
        let mut converted: Vec<Item> = items
            .iter()
            .filter_map(|d| convert::diagnostic(d, &text, encoding, &self.id, session.generation))
            .collect();
        if let Some(customizer) = &spec.customizer {
            for item in &mut converted {
                customizer.diagnostic(&mut item.diagnostic);
            }
        }
        // Under the slot's lock: a stop in between clears them for good.
        let slot = self.slot();
        if slot.generation != session.generation || slot.session.is_none() {
            return;
        }
        let cx = &self.cx;
        cx.diagnostics
            .set(&cx.documents, &cx.hub, doc, &self.id, version, converted);
        drop(slot);
    }

    /// `workspace/diagnostic/refresh`: every open document's diagnostics
    /// are pulled again in full.
    pub(super) fn refresh_diagnostics(&self, generation: u64) {
        let Some(session) = self.session_of(generation) else {
            return;
        };
        lock(&session.result_ids).clear();
        let docs: Vec<DocumentId> = lock(&session.docs).keys().cloned().collect();
        for doc in docs {
            self.schedule_pull(&session, &doc, Duration::ZERO);
        }
    }
}
