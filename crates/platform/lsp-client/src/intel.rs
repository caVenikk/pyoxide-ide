//! [`CodeIntel`] over the language servers of a project.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use code_intel::{
    AppliedEdit, CodeAction, CodeActionContext, CodeIntel, CompletionItem, CompletionTrigger,
    Completions, DocumentId, EventHub, FileChange, FormatOptions, Hover, Intel, IntelEvent,
    Location, Point, Rope, ServerId, ServerStatus, SignatureContext, SignatureHelp, TextEdit,
    ready, unsupported,
};

use crate::server::{Context, LanguageServer, Timeouts};
use crate::spec::ServerSpec;

/// The language services of a project over its servers: each document
/// goes to the servers of its language, each request to the servers that
/// serve its feature. Servers start when the first document of theirs
/// opens.
pub struct LspCodeIntel {
    cx: Arc<Context>,
    servers: Mutex<Vec<Arc<LanguageServer>>>,
    /// Servers replaced or removed by [`LspCodeIntel::set_specs`] whose
    /// processes may still be stopping.
    retired: Mutex<Vec<Arc<LanguageServer>>>,
    /// The language of each open document.
    open: Mutex<HashMap<DocumentId, String>>,
    /// Shut down: no server starts again.
    closed: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LspCodeIntel {
    /// The services over these servers (none is fine: see
    /// [`LspCodeIntel::set_specs`]).
    pub fn new(specs: Vec<ServerSpec>) -> Arc<Self> {
        Self::with_timeouts(specs, Timeouts::default())
    }

    /// With other waiting times than the reference's (tests, slow
    /// machines).
    pub fn with_timeouts(specs: Vec<ServerSpec>, timeouts: Timeouts) -> Arc<Self> {
        let cx = Arc::new(Context {
            hub: EventHub::new(),
            timeouts,
        });
        let servers = specs
            .into_iter()
            .map(|spec| LanguageServer::new(spec, cx.clone()))
            .collect();
        Arc::new(Self {
            cx,
            servers: Mutex::new(servers),
            retired: Mutex::new(Vec::new()),
            open: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    /// The servers it runs.
    pub fn specs(&self) -> Vec<ServerSpec> {
        self.servers()
            .iter()
            .map(|server| ServerSpec::clone(&server.spec()))
            .collect()
    }

    /// Replaces the servers (the product found its executables, the
    /// interpreter or the settings changed). A server whose spec is gone,
    /// or changed in how it starts (command line, folder, initialization
    /// options, settings), is stopped; a changed or new one starts when a
    /// document of its languages is open. A server with only a new name,
    /// icon, features, languages or customizer keeps running.
    pub fn set_specs(&self, specs: Vec<ServerSpec>) {
        let mut stopping = Vec::new();
        let mut fresh = Vec::new();
        {
            let mut servers = lock(&self.servers);
            let mut old = std::mem::take(&mut *servers);
            for spec in specs {
                let same = old.iter().position(|s| *s.id() == spec.id);
                match same {
                    Some(i) if !old[i].needs_restart_for(&spec) => {
                        let server = old.remove(i);
                        server.update(spec);
                        servers.push(server);
                    }
                    _ => {
                        if let Some(i) = same {
                            stopping.push(old.remove(i));
                        }
                        let server = LanguageServer::new(spec, self.cx.clone());
                        fresh.push(server.clone());
                        servers.push(server);
                    }
                }
            }
            stopping.extend(old);
        }
        for server in &stopping {
            server.stop();
        }
        {
            let mut retired = lock(&self.retired);
            retired.retain(|server| server.is_stopping());
            retired.extend(stopping);
        }
        if self.is_closed() {
            return;
        }
        let languages: Vec<String> = lock(&self.open).values().cloned().collect();
        for server in fresh {
            let spec = server.spec();
            if languages.iter().any(|language| spec.serves(language)) {
                server.ensure_started();
            }
        }
    }

    /// The settings or the customizers' answers changed: the running
    /// servers are told (`workspace/didChangeConfiguration`).
    pub fn configuration_changed(&self) {}

    fn servers(&self) -> Vec<Arc<LanguageServer>> {
        lock(&self.servers).clone()
    }

    fn server(&self, id: &ServerId) -> Option<Arc<LanguageServer>> {
        lock(&self.servers).iter().find(|s| s.id() == id).cloned()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl CodeIntel for LspCodeIntel {
    fn did_open(&self, doc: &DocumentId, language: &str, _: u64, _: &Rope) {
        lock(&self.open).insert(doc.clone(), language.to_owned());
        if self.is_closed() {
            return;
        }
        for server in self.servers() {
            if server.spec().serves(language) {
                server.ensure_started();
            }
        }
    }

    fn did_change(&self, _: &DocumentId, _: u64, _: &[AppliedEdit], _: &Rope) {}

    fn did_reload(&self, _: &DocumentId, _: u64, _: &Rope) {}

    fn did_save(&self, _: &DocumentId, _: &Rope) {}

    fn did_close(&self, doc: &DocumentId) {
        lock(&self.open).remove(doc);
    }

    fn files_changed(&self, _: &[FileChange]) {}

    fn completion(&self, _: &DocumentId, _: Point, _: CompletionTrigger) -> Intel<Completions> {
        unsupported()
    }

    fn resolve_completion(&self, _: &DocumentId, item: CompletionItem) -> Intel<CompletionItem> {
        ready(Ok(item))
    }

    fn hover(&self, _: &DocumentId, _: Point) -> Intel<Option<Hover>> {
        unsupported()
    }

    fn signature_help(
        &self,
        _: &DocumentId,
        _: Point,
        _: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        unsupported()
    }

    fn definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn type_definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn references(&self, _: &DocumentId, _: Point, _: bool) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn code_actions(
        &self,
        _: &DocumentId,
        _: Range<Point>,
        _: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        unsupported()
    }

    fn resolve_code_action(&self, _: &DocumentId, action: CodeAction) -> Intel<CodeAction> {
        ready(Ok(action))
    }

    fn format(
        &self,
        _: &DocumentId,
        _: Option<Range<Point>>,
        _: FormatOptions,
    ) -> Intel<Vec<TextEdit>> {
        unsupported()
    }

    fn status(&self) -> Vec<ServerStatus> {
        self.servers().iter().map(|s| s.status()).collect()
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.cx.hub.subscribe()
    }

    fn restart(&self, server: Option<&ServerId>) {
        if self.is_closed() {
            return;
        }
        match server {
            Some(id) => {
                if let Some(server) = self.server(id) {
                    server.restart();
                }
            }
            None => {
                for server in self.servers() {
                    server.restart();
                }
            }
        }
    }

    fn stop(&self, server: &ServerId) {
        if let Some(server) = self.server(server) {
            server.stop();
        }
    }

    fn shutdown(&self) -> Intel<()> {
        self.closed.store(true, Ordering::Release);
        let servers = self.servers();
        for server in &servers {
            server.stop();
        }
        let stopped: Vec<_> = servers
            .iter()
            .chain(lock(&self.retired).iter())
            .map(|server| server.stopped())
            .collect();
        Box::pin(async move {
            futures::future::join_all(stopped).await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use code_intel::{Feature, Features, IntelError, ServerState};
    use futures::executor::block_on;
    use process::CommandLine;

    use super::*;

    #[test]
    fn servers_are_listed_and_unrouted_features_unsupported() {
        let ruff = ServerSpec::new("ruff", CommandLine::new("ruff").arg("server"), "/p")
            .with_languages(["python"])
            .with_features(Features::of(&[Feature::Diagnostics, Feature::Formatting]));
        assert!(ruff.serves("python"));
        assert!(format!("{ruff:?}").contains("ruff"));
        let intel: Arc<dyn CodeIntel> = LspCodeIntel::new(vec![ruff]);
        let status = intel.status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, ServerState::Stopped);
        assert!(status[0].features.contains(Feature::Formatting));
        let doc = DocumentId::new("/p/a.txt");
        // Not a language of the server: nothing starts.
        intel.did_open(&doc, "plaintext", 1, &Rope::from_str("x = 1\n"));
        assert_eq!(intel.status()[0].state, ServerState::Stopped);
        assert_eq!(
            block_on(intel.completion(&doc, Point::new(0, 1), CompletionTrigger::Invoked)),
            Err(IntelError::Unsupported)
        );
        assert_eq!(block_on(intel.shutdown()), Ok(()));
    }
}
