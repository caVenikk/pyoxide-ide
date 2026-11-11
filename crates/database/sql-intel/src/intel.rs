//! [`SqlCodeIntel`]: the SQL services as a `CodeIntel` provider (ADR 0022).
//! Lifecycle calls record the text and return; one background thread
//! re-parses the changed statements, analyses what changed and publishes
//! `IntelEvent::Diagnostics`. A newer edit cancels the analysis of an older
//! text between two statements. Code actions are answered by the same
//! thread from the analysis of the current text.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread;

use code_intel::{
    AppliedEdit, CodeAction, CodeActionContext, CodeActionKind, CodeIntel, Command, CompletionItem,
    CompletionTrigger, Completions, Diagnostic, DocumentId, EventHub, Feature, Features,
    FileChange, FileEdits, FormatOptions, Hover, InsertText, Intel, IntelError, IntelEvent,
    Location, Markup, Point, Rope, ServerId, ServerState, ServerStatus, SignatureContext,
    SignatureHelp, TextEdit, Token, WorkspaceEdit, positions, ready, unsupported,
};
use futures::channel::oneshot;
use sql_syntax::{Dialect, Script};

use crate::analysis::{Analyzer, DocumentAnalysis};
use crate::complete::{self, Resolve, ScriptObjects};
use crate::context::ScopeProvider;
use crate::docs;
use crate::hover as quick_doc;
use crate::inspect::{Fix, Problem};
use crate::profile::{Inspection, InspectionProfile};
use crate::signature;

/// The id of the services in the Language Services widget.
pub const SERVER_ID: &str = "sql";

/// The data of the *Disable inspection* command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableInspection(Inspection);

/// What the UI thread hands the worker: the latest text of a document.
struct Input {
    version: u64,
    rope: Rope,
    /// Bumped when the document's dialect or scope may have changed.
    epoch: u64,
    /// The latest version, read by the worker between statements.
    latest: Arc<AtomicU64>,
}

type ProfileListener = Box<dyn Fn(&InspectionProfile) + Send + Sync>;

struct Shared {
    provider: Arc<dyn ScopeProvider>,
    hub: EventHub,
    inputs: Mutex<HashMap<DocumentId, Input>>,
    profile: RwLock<InspectionProfile>,
    on_profile: Mutex<Option<ProfileListener>>,
    analysed: Arc<AtomicUsize>,
}

impl Shared {
    fn inputs(&self) -> std::sync::MutexGuard<'_, HashMap<DocumentId, Input>> {
        self.inputs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn profile(&self) -> InspectionProfile {
        self.profile
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

enum Job {
    Changed(DocumentId),
    Closed(DocumentId),
    /// The profile changed: publish again without analysing.
    Republish,
    Actions {
        doc: DocumentId,
        range: Range<Point>,
        reply: oneshot::Sender<Result<Vec<CodeAction>, IntelError>>,
    },
    Completion {
        doc: DocumentId,
        at: Point,
        reply: oneshot::Sender<Result<Completions, IntelError>>,
    },
    Hover {
        doc: DocumentId,
        at: Point,
        reply: oneshot::Sender<Result<Option<Hover>, IntelError>>,
    },
    Signature {
        doc: DocumentId,
        at: Point,
        reply: oneshot::Sender<Result<Option<SignatureHelp>, IntelError>>,
    },
    Shutdown(oneshot::Sender<()>),
}

/// The SQL language services for documents of the language id `"sql"`.
pub struct SqlCodeIntel {
    shared: Arc<Shared>,
    jobs: flume::Sender<Job>,
}

impl SqlCodeIntel {
    /// Services that ask `provider` for the dialect and scope of each
    /// document, with the default inspection profile.
    pub fn new(provider: Arc<dyn ScopeProvider>) -> Self {
        Self::with_profile(provider, InspectionProfile::default())
    }

    pub fn with_profile(provider: Arc<dyn ScopeProvider>, profile: InspectionProfile) -> Self {
        let shared = Arc::new(Shared {
            provider,
            hub: EventHub::new(),
            inputs: Mutex::new(HashMap::new()),
            profile: RwLock::new(profile),
            on_profile: Mutex::new(None),
            analysed: Arc::new(AtomicUsize::new(0)),
        });
        let (jobs, rx) = flume::unbounded();
        let worker = Worker {
            shared: shared.clone(),
            docs: HashMap::new(),
            pending: rx.clone(),
            retry: Vec::new(),
        };
        thread::Builder::new()
            .name("sql-intel".into())
            .spawn(move || worker.run(rx))
            .expect("spawn the SQL analysis thread");
        Self { shared, jobs }
    }

    /// The dialect or scope of a document (or of every document) may have
    /// changed: a binding, a mapping, a refreshed model. Its analysis runs
    /// again.
    pub fn context_changed(&self, doc: Option<&DocumentId>) {
        let docs: Vec<DocumentId> = {
            let mut inputs = self.shared.inputs();
            inputs
                .iter_mut()
                .filter(|(d, _)| doc.is_none_or(|x| x == *d))
                .map(|(d, input)| {
                    input.epoch += 1;
                    d.clone()
                })
                .collect()
        };
        for d in docs {
            let _ = self.jobs.send(Job::Changed(d));
        }
    }

    pub fn profile(&self) -> InspectionProfile {
        self.shared.profile()
    }

    /// Replaces the inspection profile (the settings changed) and publishes
    /// every document's problems again.
    pub fn set_profile(&self, profile: InspectionProfile) {
        *self
            .shared
            .profile
            .write()
            .unwrap_or_else(PoisonError::into_inner) = profile;
        let _ = self.jobs.send(Job::Republish);
    }

    /// Called with the new profile when *Disable inspection* changes it, so
    /// the product writes the setting.
    pub fn on_profile_change(&self, listener: impl Fn(&InspectionProfile) + Send + Sync + 'static) {
        *self
            .shared
            .on_profile
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(listener));
    }

    /// How many statements were analysed (cache misses), over all
    /// documents.
    pub fn analysed(&self) -> usize {
        self.shared.analysed.load(Ordering::Relaxed)
    }

    fn changed(&self, doc: &DocumentId, version: u64, text: &Rope) {
        {
            let mut inputs = self.shared.inputs();
            let input = inputs.entry(doc.clone()).or_insert_with(|| Input {
                version,
                rope: text.clone(),
                epoch: 0,
                latest: Arc::new(AtomicU64::new(version)),
            });
            input.version = version;
            input.rope = text.clone();
            input.latest.store(version, Ordering::Release);
        }
        let _ = self.jobs.send(Job::Changed(doc.clone()));
    }
}

impl Drop for SqlCodeIntel {
    fn drop(&mut self) {
        let (tx, _rx) = oneshot::channel();
        let _ = self.jobs.send(Job::Shutdown(tx));
    }
}

impl CodeIntel for SqlCodeIntel {
    fn did_open(&self, doc: &DocumentId, _language: &str, version: u64, text: &Rope) {
        self.changed(doc, version, text);
    }

    fn did_change(&self, doc: &DocumentId, version: u64, _edits: &[AppliedEdit], text: &Rope) {
        // The worker finds the changed part itself (it parses at its own
        // pace and may skip versions).
        self.changed(doc, version, text);
    }

    fn did_reload(&self, doc: &DocumentId, version: u64, text: &Rope) {
        self.changed(doc, version, text);
    }

    fn did_save(&self, _doc: &DocumentId, _text: &Rope) {}

    fn did_close(&self, doc: &DocumentId) {
        self.shared.inputs().remove(doc);
        let _ = self.jobs.send(Job::Closed(doc.clone()));
    }

    fn files_changed(&self, _changes: &[FileChange]) {}

    fn completion(&self, doc: &DocumentId, at: Point, _: CompletionTrigger) -> Intel<Completions> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Completion {
            doc: doc.clone(),
            at,
            reply,
        };
        if self.jobs.send(job).is_err() {
            return ready(Err(IntelError::NotRunning));
        }
        Box::pin(async move { rx.await.unwrap_or(Err(IntelError::Cancelled)) })
    }

    fn resolve_completion(
        &self,
        doc: &DocumentId,
        mut item: CompletionItem,
    ) -> Intel<CompletionItem> {
        let Some(resolve) = item
            .token
            .as_ref()
            .and_then(|t| t.downcast_ref::<Resolve>())
        else {
            return ready(Ok(item));
        };
        let ctx = self.shared.provider.context(doc);
        let text = match resolve {
            Resolve::Node(id) => ctx
                .scope
                .as_ref()
                .and_then(|s| docs::node_doc(&s.model, *id, ctx.dialect)),
            Resolve::Function(f) => {
                let overloads = crate::builtins::function(ctx.dialect, f.name);
                Some(docs::function_doc(f, &overloads))
            }
            Resolve::Type(t) => Some(docs::type_doc(t)),
            Resolve::Keyword(k) => docs::keyword_text(k),
        };
        item.documentation = text.map(Markup::markdown);
        ready(Ok(item))
    }

    fn completion_triggers(&self, _: &DocumentId) -> Vec<char> {
        vec!['.']
    }

    fn hover(&self, doc: &DocumentId, at: Point) -> Intel<Option<Hover>> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Hover {
            doc: doc.clone(),
            at,
            reply,
        };
        if self.jobs.send(job).is_err() {
            return ready(Err(IntelError::NotRunning));
        }
        Box::pin(async move { rx.await.unwrap_or(Err(IntelError::Cancelled)) })
    }

    fn signature_help(
        &self,
        doc: &DocumentId,
        at: Point,
        _: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Signature {
            doc: doc.clone(),
            at,
            reply,
        };
        if self.jobs.send(job).is_err() {
            return ready(Err(IntelError::NotRunning));
        }
        Box::pin(async move { rx.await.unwrap_or(Err(IntelError::Cancelled)) })
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
        doc: &DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        if !context.wants(&CodeActionKind::QuickFix) {
            return ready(Ok(Vec::new()));
        }
        let (reply, rx) = oneshot::channel();
        if self
            .jobs
            .send(Job::Actions {
                doc: doc.clone(),
                range,
                reply,
            })
            .is_err()
        {
            return ready(Err(IntelError::NotRunning));
        }
        Box::pin(async move { rx.await.unwrap_or(Err(IntelError::Cancelled)) })
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

    fn execute_command(&self, command: Command) -> Intel<()> {
        let Some(DisableInspection(inspection)) = command.token.downcast_ref().copied() else {
            return unsupported();
        };
        let profile = {
            let mut p = self
                .shared
                .profile
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            p.disable(inspection);
            p.clone()
        };
        if let Some(listener) = self
            .shared
            .on_profile
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            listener(&profile);
        }
        let _ = self.jobs.send(Job::Republish);
        ready(Ok(()))
    }

    fn status(&self) -> Vec<ServerStatus> {
        let mut status = ServerStatus::new(ServerId::new(SERVER_ID), "SQL");
        status.state = ServerState::Running;
        status.features = Features::of(&[
            Feature::Diagnostics,
            Feature::CodeActions,
            Feature::Completion,
            Feature::Hover,
            Feature::SignatureHelp,
        ]);
        status.documents = self.shared.inputs().keys().cloned().collect();
        status.documents.sort();
        vec![status]
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.shared.hub.subscribe()
    }

    fn restart(&self, server: Option<&ServerId>) {
        if server.is_none_or(|s| s.as_str() == SERVER_ID) {
            self.context_changed(None);
        }
    }

    fn stop(&self, _server: &ServerId) {}

    fn shutdown(&self) -> Intel<()> {
        // The worker ends by itself; the application does not wait for it
        // (a quitting application parks nothing on a thread of ours).
        let (tx, _rx) = oneshot::channel();
        let _ = self.jobs.send(Job::Shutdown(tx));
        ready(Ok(()))
    }
}

/// A document as the worker has it.
struct Work {
    version: u64,
    epoch: u64,
    rope: Rope,
    script: Script,
    analyzer: Analyzer,
    /// The analysis of `script`; `None` when it was cancelled.
    analysis: Option<DocumentAnalysis>,
    /// What the script creates, for completion; built on first use.
    objects: Option<ScriptObjects>,
}

struct Worker {
    shared: Arc<Shared>,
    docs: HashMap<DocumentId, Work>,
    /// The queue of jobs, to see whether somebody waits (an analysis gives
    /// way to a request).
    pending: flume::Receiver<Job>,
    /// Documents whose analysis gave way to a request.
    retry: Vec<DocumentId>,
}

impl Worker {
    fn run(mut self, rx: flume::Receiver<Job>) {
        loop {
            let mut jobs = Vec::new();
            if self.retry.is_empty() {
                match rx.recv() {
                    Ok(first) => jobs.push(first),
                    Err(_) => break,
                }
            }
            jobs.extend(rx.try_iter());
            let mut changed: Vec<DocumentId> = std::mem::take(&mut self.retry);
            let mut republish = false;
            let mut actions = Vec::new();
            let mut asks = Vec::new();
            let mut shutdown = Vec::new();
            for job in jobs {
                match job {
                    Job::Changed(d) => {
                        if !changed.contains(&d) {
                            changed.push(d);
                        }
                    }
                    Job::Closed(d) => {
                        changed.retain(|c| *c != d);
                        if self.docs.remove(&d).is_some() {
                            self.shared.hub.emit(IntelEvent::Diagnostics {
                                doc: d,
                                version: 0,
                                items: Vec::new(),
                            });
                        }
                    }
                    Job::Republish => republish = true,
                    a @ Job::Actions { .. } => actions.push(a),
                    a @ (Job::Completion { .. } | Job::Hover { .. } | Job::Signature { .. }) => {
                        asks.push(a);
                    }
                    Job::Shutdown(tx) => shutdown.push(tx),
                }
            }
            if !shutdown.is_empty() {
                for tx in shutdown {
                    let _ = tx.send(());
                }
                return;
            }
            // Somebody waits for these: before the analysis of changes.
            for job in asks {
                self.answer(job);
            }
            for d in &changed {
                self.update(d, true);
            }
            if republish {
                let docs: Vec<DocumentId> = self.docs.keys().cloned().collect();
                for d in docs {
                    if !changed.contains(&d) {
                        self.publish(&d);
                    }
                }
            }
            for job in actions {
                if let Job::Actions { doc, range, reply } = job {
                    self.update(&doc, false);
                    let _ = reply.send(Ok(self.actions(&doc, range)));
                }
            }
        }
    }

    /// Brings the analysis of `doc` to its latest text and publishes it.
    ///
    /// With `yielding` the analysis stops when a request is waiting and
    /// goes on after it.
    fn update(&mut self, doc: &DocumentId, yielding: bool) {
        if self.sync(doc) {
            self.analyse(doc, yielding);
        }
    }

    /// Brings the script of `doc` to its latest text (parsing what changed);
    /// `false` when the document is not open.
    fn sync(&mut self, doc: &DocumentId) -> bool {
        let (version, rope, epoch) = {
            let inputs = self.shared.inputs();
            let Some(input) = inputs.get(doc) else {
                return false;
            };
            (input.version, input.rope.clone(), input.epoch)
        };
        if let Some(w) = self.docs.get(doc)
            && w.version == version
            && w.epoch == epoch
        {
            return true;
        }
        let ctx = self.shared.provider.context(doc);
        let text = rope.to_string();
        let work = match self.docs.remove(doc) {
            Some(mut w) => {
                w.script = next_script(&w.script, &text, ctx.dialect);
                w.version = version;
                w.epoch = epoch;
                w.rope = rope;
                w.analysis = None;
                w.objects = None;
                w
            }
            None => Work {
                version,
                epoch,
                rope,
                script: sql_syntax::parse(&text, ctx.dialect),
                analyzer: Analyzer::with_counter(self.shared.analysed.clone()),
                analysis: None,
                objects: None,
            },
        };
        self.docs.insert(doc.clone(), work);
        true
    }

    /// Analyses the script of `doc` (when that is not done) and publishes
    /// the problems.
    fn analyse(&mut self, doc: &DocumentId, yielding: bool) {
        let latest = {
            let inputs = self.shared.inputs();
            let Some(input) = inputs.get(doc) else {
                return;
            };
            input.latest.clone()
        };
        let Some(mut work) = self.docs.remove(doc) else {
            return;
        };
        let mut fresh = false;
        if work.analysis.is_none() {
            let ctx = self.shared.provider.context(doc);
            let version = work.version;
            let pending = &self.pending;
            let cancel =
                || latest.load(Ordering::Acquire) != version || (yielding && !pending.is_empty());
            work.analysis = work.analyzer.analyze(&work.script, &ctx, &cancel);
            fresh = work.analysis.is_some();
            if !fresh && yielding && latest.load(Ordering::Acquire) == version {
                // Somebody waits for an answer: this goes on afterwards.
                self.retry.push(doc.clone());
            }
        }
        self.docs.insert(doc.clone(), work);
        if fresh {
            self.publish(doc);
        }
    }

    /// Answers a request for completion, quick documentation or parameter
    /// info.
    fn answer(&mut self, job: Job) {
        match job {
            Job::Completion { doc, at, reply } => {
                let _ = reply.send(self.complete(&doc, at));
            }
            Job::Hover { doc, at, reply } => {
                let _ = reply.send(self.hover(&doc, at));
            }
            Job::Signature { doc, at, reply } => {
                let _ = reply.send(self.signature(&doc, at));
            }
            _ => {}
        }
    }

    fn complete(&mut self, doc: &DocumentId, at: Point) -> Result<Completions, IntelError> {
        if !self.sync(doc) {
            return Err(IntelError::Unsupported);
        }
        let ctx = self.shared.provider.context(doc);
        let Some(w) = self.docs.get_mut(doc) else {
            return Err(IntelError::Unsupported);
        };
        let offset = point_to_byte(&w.rope, at);
        if w.objects.is_none() {
            w.objects = Some(ScriptObjects::build(&w.script, &ctx));
        }
        let objects = w.objects.as_ref().expect("built above");
        let answer = complete::complete(&w.script, &ctx, objects, offset);
        Ok(completions(&w.rope, offset, answer))
    }

    fn hover(&mut self, doc: &DocumentId, at: Point) -> Result<Option<Hover>, IntelError> {
        self.update(doc, false);
        let ctx = self.shared.provider.context(doc);
        let Some(w) = self.docs.get(doc) else {
            return Err(IntelError::Unsupported);
        };
        let Some(analysis) = &w.analysis else {
            return Ok(None);
        };
        let offset = point_to_byte(&w.rope, at);
        Ok(
            quick_doc::hover(&w.script, &ctx, analysis, offset).map(|d| Hover {
                contents: Markup::markdown(d.text),
                range: Some(
                    byte_to_point(&w.rope, d.range.start)..byte_to_point(&w.rope, d.range.end),
                ),
            }),
        )
    }

    fn signature(
        &mut self,
        doc: &DocumentId,
        at: Point,
    ) -> Result<Option<SignatureHelp>, IntelError> {
        if !self.sync(doc) {
            return Err(IntelError::Unsupported);
        }
        let ctx = self.shared.provider.context(doc);
        let Some(w) = self.docs.get(doc) else {
            return Err(IntelError::Unsupported);
        };
        let offset = point_to_byte(&w.rope, at);
        Ok(signature::signature_help(&w.script, &ctx, offset))
    }

    fn publish(&self, doc: &DocumentId) {
        let Some(w) = self.docs.get(doc) else {
            return;
        };
        let Some(analysis) = &w.analysis else {
            return;
        };
        let profile = self.shared.profile();
        let items = analysis
            .problems(&profile)
            .iter()
            .filter_map(|p| diagnostic(p, &profile, &w.rope))
            .collect();
        self.shared.hub.emit(IntelEvent::Diagnostics {
            doc: doc.clone(),
            version: w.version,
            items,
        });
    }

    fn actions(&self, doc: &DocumentId, range: Range<Point>) -> Vec<CodeAction> {
        let Some(w) = self.docs.get(doc) else {
            return Vec::new();
        };
        let Some(analysis) = &w.analysis else {
            return Vec::new();
        };
        let rope = &w.rope;
        let start = point_to_byte(rope, range.start);
        let end = point_to_byte(rope, range.end).max(start);
        let profile = self.shared.profile();
        let text = w.script.text();
        let mut out = Vec::new();
        for p in analysis.problems(&profile) {
            let touches = p.range.start <= end && start <= p.range.end;
            if !touches {
                continue;
            }
            let Some(diag) = diagnostic(&p, &profile, rope) else {
                continue;
            };
            let mut options = Vec::new();
            if p.inspection.suppressible() {
                if let Some(fix) = suppress_fix(analysis, text, &p) {
                    options.push(code_action(doc, rope, &fix, &diag, false));
                }
                let title = format!("Disable inspection '{}'", p.inspection.title());
                let mut disable = CodeAction::new(title.clone(), CodeActionKind::QuickFix);
                disable.command = Some(Command {
                    title,
                    token: Token::new(DisableInspection(p.inspection)),
                });
                disable.diagnostics = vec![diag.clone()];
                options.push(disable);
            }
            if p.fixes.is_empty() {
                out.extend(options);
                continue;
            }
            for (i, fix) in p.fixes.iter().enumerate() {
                let mut action = code_action(doc, rope, fix, &diag, i == 0);
                action.children = options.clone();
                out.push(action);
            }
        }
        out
    }
}

/// The script of `text`: the old one re-parsed where the texts differ.
fn next_script(old: &Script, text: &str, dialect: Dialect) -> Script {
    if old.dialect() != dialect {
        return sql_syntax::parse(text, dialect);
    }
    let a = old.text().as_bytes();
    let b = text.as_bytes();
    let mut prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let mut suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suffix)
        .take_while(|(x, y)| x == y)
        .count();
    if prefix == a.len() && prefix == b.len() {
        return old.clone();
    }
    while !text.is_char_boundary(prefix) || !old.text().is_char_boundary(prefix) {
        prefix -= 1;
    }
    while !text.is_char_boundary(b.len() - suffix) || !old.text().is_char_boundary(a.len() - suffix)
    {
        suffix -= 1;
    }
    old.reparse(&sql_syntax::TextEdit {
        range: prefix..a.len() - suffix,
        text: text[prefix..b.len() - suffix].to_string(),
    })
}

/// The `code-intel` answer for the engine's items.
fn completions(rope: &Rope, caret: usize, answer: complete::Answer) -> Completions {
    let insert = byte_to_point(rope, answer.start)..byte_to_point(rope, caret);
    let replace =
        byte_to_point(rope, answer.start)..byte_to_point(rope, answer.word_end.max(caret));
    let server = ServerId::new(SERVER_ID);
    let items = answer
        .items
        .into_iter()
        .map(|i| {
            let mut item = CompletionItem::new(i.label).with_kind(i.kind);
            item.insert = InsertText::Edit {
                insert: insert.clone(),
                replace: replace.clone(),
                text: i.insert,
            };
            item.snippet = i.snippet;
            item.tail_text = i.tail;
            item.type_text = i.detail;
            item.filter_text = i.filter;
            item.sort_text = Some(i.sort);
            item.deprecated = i.deprecated;
            item.server = Some(server.clone());
            item.token = i.resolve.map(Token::new);
            item
        })
        .collect();
    Completions {
        items,
        incomplete: answer.incomplete,
    }
}

fn byte_to_point(rope: &Rope, byte: usize) -> Point {
    let byte = byte.min(rope.len_bytes());
    let c = rope.byte_to_char(byte);
    positions::offset_to_point(rope, c)
}

fn point_to_byte(rope: &Rope, at: Point) -> usize {
    rope.char_to_byte(positions::point_to_offset(rope, at))
}

fn diagnostic(p: &Problem, profile: &InspectionProfile, rope: &Rope) -> Option<Diagnostic> {
    let severity = profile.level(p.inspection).severity()?;
    let mut d = Diagnostic::new(
        byte_to_point(rope, p.range.start)..byte_to_point(rope, p.range.end),
        severity,
        p.message.clone(),
    );
    if p.inspection != Inspection::Syntax {
        d.code = Some(p.inspection.id().to_string());
    }
    d.unnecessary = p.unnecessary;
    d.server = Some(ServerId::new(SERVER_ID));
    Some(d)
}

fn code_action(
    doc: &DocumentId,
    rope: &Rope,
    fix: &Fix,
    diag: &Diagnostic,
    preferred: bool,
) -> CodeAction {
    let mut action = CodeAction::new(fix.title.clone(), CodeActionKind::QuickFix);
    action.preferred = preferred;
    action.diagnostics = vec![diag.clone()];
    action.server = Some(ServerId::new(SERVER_ID));
    action.edit = Some(WorkspaceEdit {
        files: vec![FileEdits {
            path: doc.path().to_owned(),
            edits: fix
                .edits
                .iter()
                .map(|e| {
                    TextEdit::new(
                        byte_to_point(rope, e.range.start)..byte_to_point(rope, e.range.end),
                        e.text.clone(),
                    )
                })
                .collect(),
        }],
    });
    action
}

/// *Suppress for statement*: `-- noinspection <id>` on a line of its own
/// before the statement, or the id added to such a comment.
fn suppress_fix(analysis: &DocumentAnalysis, text: &str, p: &Problem) -> Option<Fix> {
    let id = p.inspection.id();
    let statement = analysis.statement_at(p.range.start)?;
    if let Some((ids, end)) = analysis.suppression_comment(text, p.range.start) {
        if ids.iter().any(|i| i == id) {
            return None;
        }
        return Some(Fix {
            title: "Suppress for statement".into(),
            edits: vec![crate::inspect::FixEdit {
                range: end..end,
                text: format!(",{id}"),
            }],
        });
    }
    let line_start = text[..statement.start].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..statement.start];
    let (at, insert) = if before.chars().all(char::is_whitespace) {
        (line_start, format!("{before}-- noinspection {id}\n"))
    } else {
        (statement.start, format!("\n-- noinspection {id}\n"))
    };
    Some(Fix {
        title: "Suppress for statement".into(),
        edits: vec![crate::inspect::FixEdit {
            range: at..at,
            text: insert,
        }],
    })
}
