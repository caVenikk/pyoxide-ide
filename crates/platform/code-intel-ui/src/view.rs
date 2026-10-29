//! The code intelligence of one main window: the controller between the
//! editors of its tabs and the project's language services.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::{
    AppliedEdit, CodeIntel, CompletionTrigger, Completions, DocumentId, Feature, FileChange,
    FileChangeKind, IntelError, IntelEvent, ObjectTarget, Route, RoutedCodeIntel, ServerId,
    ServerStatus,
};
use editor::{Analysis, Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Entity, EntityId, EventEmitter,
    Focusable as _, InteractiveElement, SharedString, Subscription, Task, Window,
};
use serde_json::{Value as Json, json};
use text::{CaretPlacement, Edit, EditKind, Point, SelectionSet};
use themes::TextRole;

use crate::actions::Reported;
use crate::commands::*;
use crate::completion::list::CompletionList;
use crate::completion::{self, Session, identifier_start, is_identifier_char};
use crate::doc::{self, DocPopup};
use crate::doc_window::DocToolWindow;
use crate::document::Document;
use crate::hook::CodeIntelHook;
use crate::hover::DocState;
use crate::intentions::IntentionState;
use crate::marks;
use crate::navigation::NavState;
use crate::occurrences::OccurrenceState;
use crate::popup::{CaretPopup, Placement, PopupHost, TextHint};
use crate::rename::RenameState;
use crate::services::{self, InlineAction, ServerItem, ServicesPopup, WidgetBounds};
use crate::signature::{self, SignaturePopup, SignatureState};

/// The key context of the hint shown when Basic Completion finds nothing.
pub const HINT_CONTEXT: &str = "CompletionHint";
/// The hint (`completion.no.suggestions` of the reference).
pub const NO_SUGGESTIONS: &str = "No suggestions";

/// Where a request about the caret of an editor is made: the document,
/// the caret as a char offset and as a position, and the version of the
/// text they are in.
#[derive(Debug, Clone)]
pub struct CaretQuery {
    pub path: PathBuf,
    pub doc: DocumentId,
    pub offset: usize,
    pub at: Point,
    pub version: u64,
}

/// What the window does for the code intelligence.
#[derive(Debug, Clone, PartialEq)]
pub enum CodeIntelEvent {
    /// A popup appeared or changed without a key press (an answer came):
    /// it should be drawn at once, not at the next refresh.
    Redraw,
    /// A notice for the status bar (the reference shows a balloon).
    Notice(String),
    /// Open a file in a tab.
    OpenFile(PathBuf),
    /// Open a text that is no file in a read-only tab titled `title`.
    OpenText { title: String, text: String },
    /// Open a file in a tab with the caret at a position (a navigation
    /// target).
    OpenLocation { path: PathBuf, at: Point },
    /// Open an object that is no place in a file (a database table): the
    /// handler registered for its scheme does it.
    OpenObject(ObjectTarget),
    /// Show the Documentation tool window ([`CodeIntelView::doc_window`])
    /// without moving the focus, registering it at its first use.
    ShowDocumentation,
    /// Open the Rename dialog for the symbol at the caret of the active
    /// editor, with its name; the dialog reports the new name through
    /// [`CodeIntelView::rename_confirmed`] or
    /// [`CodeIntelView::rename_cancelled`].
    RenameDialog { name: String },
    /// The services refused the new name (a message for the dialog).
    RenameRejected(String),
    /// The rename was applied (the dialog closes).
    RenameDone,
}

/// The code intelligence of a main window: the project's language
/// services, the documents of the window's editor tabs and the popups over
/// them. The workbench reports the project, editors, saves and the active
/// tab; the view does the rest.
pub struct CodeIntelView {
    /// The providers, the product's first (ADR 0022).
    hooks: Vec<CodeIntelHook>,
    /// The services of the open project: the one provider's, or all routed
    /// by language.
    services: Option<Arc<dyn CodeIntel>>,
    /// Each provider's services, in the order of `hooks`.
    parts: Vec<Arc<dyn CodeIntel>>,
    root: Option<PathBuf>,
    /// The documents, by path.
    documents: HashMap<PathBuf, Document>,
    /// The file of the active tab.
    active: Option<PathBuf>,
    popups: PopupHost,
    completion: Option<Session>,
    completion_list: Entity<CompletionList>,
    /// Completion sessions started so far (their ids).
    sessions: u64,
    /// An automatic popup found nothing for the identifier that starts
    /// here (editor, offset): typing on in it opens none (the reference's
    /// `EmptyAutoPopup` phase).
    empty_auto: Option<(EntityId, usize)>,
    /// The Language Services popup, while it is open.
    services_popup: Option<(Entity<ServicesPopup>, Subscription)>,
    widget_bounds: WidgetBounds,
    /// Quick documentation, the popup over the text, Error Description.
    pub(crate) docs: DocState,
    /// Parameter info.
    pub(crate) signatures: SignatureState,
    /// The diagnostics of the documents as the services reported them.
    pub(crate) reported: Reported,
    /// Go to Declaration, Show Usages and the list popups.
    pub(crate) nav: NavState,
    /// Highlight Usages in File.
    pub(crate) occurrences: OccurrenceState,
    /// Rename.
    pub(crate) rename: RenameState,
    /// Show Context Actions and the bulb.
    pub(crate) intentions: IntentionState,
    /// Reformat Code or Optimize Imports in flight.
    pub(crate) format_request: Option<Task<()>>,
    /// The servers' work in the status bar.
    server_tasks: crate::server_tasks::ServerTasks,
    /// Receives the services' events.
    _events: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CodeIntelEvent> for CodeIntelView {}

impl CodeIntelView {
    /// The view of a window with the services of these providers (at
    /// least one).
    pub fn new(hooks: Vec<CodeIntelHook>, cx: &mut Context<Self>) -> Self {
        debug_assert!(!hooks.is_empty(), "a provider of language services");
        // Servers do not outlive the application.
        let quit = cx.on_app_quit(|this: &mut Self, cx| {
            let services = this.services.take();
            cx.background_spawn(async move {
                if let Some(services) = services {
                    let _ = services.shutdown().await;
                }
            })
        });
        // Any key closes a hint.
        let keys = cx.observe_keystrokes(|this, _, _, cx| {
            if this.popups.hide(HINT_CONTEXT) {
                cx.notify();
            }
        });
        let owner = cx.weak_entity();
        let completion_list = cx.new(|_| CompletionList::new(owner.clone()));
        let doc_popup = cx.new(|_| DocPopup::new(owner.clone()));
        let doc_window = cx.new(|cx| DocToolWindow::new(owner.clone(), cx));
        crate::hover::follow_doc_settings(&owner, cx);
        let signature_popup = cx.new(|_| SignaturePopup::new(owner));
        Self {
            hooks,
            services: None,
            parts: Vec::new(),
            root: None,
            documents: HashMap::new(),
            active: None,
            popups: PopupHost::default(),
            completion: None,
            completion_list,
            sessions: 0,
            empty_auto: None,
            services_popup: None,
            widget_bounds: WidgetBounds::default(),
            docs: DocState::new(doc_popup, doc_window),
            signatures: SignatureState::new(signature_popup),
            reported: Reported::default(),
            nav: NavState::default(),
            occurrences: OccurrenceState::default(),
            rename: RenameState::default(),
            intentions: IntentionState::new(cx.weak_entity()),
            format_request: None,
            server_tasks: Default::default(),
            _events: None,
            _subscriptions: vec![quit, keys],
        }
    }

    /// The providers, the product's first.
    pub fn hooks(&self) -> &[CodeIntelHook] {
        &self.hooks
    }

    /// The provider that answers for the UI about a language: the first
    /// that names it, else the first that serves every language, else the
    /// product's.
    pub fn hook_for(&self, language: &str) -> &CodeIntelHook {
        let hooks = &self.hooks;
        hooks
            .iter()
            .find(|h| h.languages().contains(&language))
            .or_else(|| hooks.iter().find(|h| h.languages().is_empty()))
            .unwrap_or(&hooks[0])
    }

    /// The provider whose services run a server (else the product's).
    fn hook_of(&self, server: &ServerStatus) -> &CodeIntelHook {
        if self.parts.len() > 1
            && let Some(i) = self
                .parts
                .iter()
                .position(|p| p.status().iter().any(|s| s.id == server.id))
        {
            return &self.hooks[i];
        }
        &self.hooks[0]
    }

    /// The language services of the open project.
    pub fn services(&self) -> Option<&Arc<dyn CodeIntel>> {
        self.services.as_ref()
    }

    /// The project folder whose services these are.
    pub fn project_root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The document of a file open in a tab.
    pub fn document(&self, path: &Path) -> Option<&Document> {
        self.documents.get(path)
    }

    /// The document whose editor this is.
    pub fn document_of(&self, editor: &Entity<Editor>) -> Option<&Document> {
        self.documents
            .values()
            .find(|d| d.editor.entity_id() == editor.entity_id())
    }

    /// The documents of the window's tabs.
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.documents.values()
    }

    /// The document of the active tab.
    pub fn active_document(&self) -> Option<&Document> {
        self.documents.get(self.active.as_deref()?)
    }

    /// The caret popups (for the code intelligence features).
    pub fn popups(&self) -> &PopupHost {
        &self.popups
    }

    pub fn popups_mut(&mut self) -> &mut PopupHost {
        &mut self.popups
    }

    /// Where a request about the caret of a document's editor is made;
    /// `None` when the editor is not a document or has several carets.
    pub fn caret_query(&self, editor: &Entity<Editor>, cx: &App) -> Option<CaretQuery> {
        let doc = self.document_of(editor)?;
        let e = editor.read(cx);
        if e.selections().len() != 1 {
            return None;
        }
        let offset = e.selections().primary().head;
        Some(CaretQuery {
            path: doc.id.path().to_owned(),
            doc: doc.id.clone(),
            offset,
            at: e.buffer().offset_to_point(offset),
            version: doc.version(),
        })
    }

    // ---- project and documents ---------------------------------------------

    /// A project was opened in the window: its services replace the
    /// previous project's, which shut down. The editors of the previous
    /// project are gone (no `editor_closed` for them).
    pub fn project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        if let Some(old) = self.services.take() {
            cx.background_spawn(old.shutdown()).detach();
        }
        self.close_completion(cx);
        self.close_doc(cx);
        self.reset_signature(cx);
        self.reset_navigation(cx);
        self.clear_bulb(cx);
        self.reported.clear();
        self.popups.clear();
        self.documents.clear();
        self.services_popup = None;
        self.server_tasks.clear();
        self.active = None;
        self.root = Some(root.to_owned());
        self.parts = self.hooks.iter().map(|h| h.0.start(root, cx)).collect();
        let services = match self.parts.as_slice() {
            [one] => one.clone(),
            parts => {
                let routes = self.hooks.iter().zip(parts);
                let routes = routes.map(|(h, s)| Route::new(h.languages(), s.clone()));
                Arc::new(RoutedCodeIntel::new(routes.collect())) as Arc<dyn CodeIntel>
            }
        };
        let events = services.events();
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                if this.update(cx, |v, cx| v.service_event(event, cx)).is_err() {
                    break;
                }
            }
        }));
        self.services = Some(services);
        cx.notify();
    }

    /// An editor tab was opened for a file of this language (`None` when
    /// its file type has no language id: no services).
    pub fn editor_opened(
        &mut self,
        path: &Path,
        language: Option<&str>,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_opened(path, editor, window, cx);
        let (Some(language), Some(services)) = (language, self.services.clone()) else {
            return;
        };
        if self.documents.contains_key(path) {
            self.editor_closed(path, cx);
        }
        let key = path.to_owned();
        let events = cx.subscribe(editor, {
            let key = key.clone();
            move |this, _, event: &EditorEvent, cx| this.editor_event(&key, event, cx)
        });
        let view = cx.weak_entity();
        editor.update(cx, |e, _| {
            e.set_type_hook(Some(Rc::new(move |editor, c, cx| {
                let plan = view
                    .update(cx, |this, cx| this.completion_char(&key, editor, c, cx))
                    .ok()
                    .flatten();
                let Some(plan) = plan else {
                    return false;
                };
                editor.edit(
                    plan.edits,
                    plan.kind,
                    CaretPlacement::Explicit(SelectionSet::caret(plan.caret)),
                    cx,
                );
                plan.consumed
            })));
        });
        let blur = cx.on_blur(&editor.focus_handle(cx), window, {
            let editor = editor.entity_id();
            move |this, _, cx| this.editor_blurred(editor, cx)
        });
        // The bulb shows only while its editor has the focus.
        let focus = cx.on_focus(&editor.focus_handle(cx), window, {
            let editor = editor.entity_id();
            move |this, _, cx| this.bulb_focus_changed(editor, cx)
        });
        let id = DocumentId::new(path);
        let text = editor.read(cx).buffer().rope().clone();
        let subscriptions = vec![events, blur, focus];
        let doc = Document::new(id, language, editor.clone(), text, subscriptions);
        services.did_open(&doc.id, language, doc.version(), doc.versions.text());
        self.documents.insert(path.to_owned(), doc);
        self.update_analysis(path, cx);
        cx.notify();
    }

    /// The editor tab of a file was closed.
    pub fn editor_closed(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.remove(path) else {
            return;
        };
        if self.completion.as_ref().is_some_and(|s| s.path == path) {
            self.close_completion(cx);
        }
        if self
            .popups
            .get(doc::CONTEXT)
            .is_some_and(|p| p.editor == doc.editor)
        {
            self.close_doc(cx);
        }
        self.signature_file_closed(path, cx);
        self.reported.forget(path);
        self.popups.hide_editor(doc.editor.entity_id());
        if let Some(services) = &self.services {
            services.did_close(&doc.id);
        }
        cx.notify();
    }

    /// The file of an editor tab was written to disk: the services see the
    /// save of a document and the change of the file (servers that watch
    /// files, such as configuration files, reload them).
    pub fn file_saved(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(services) = &self.services else {
            return;
        };
        if let Some(doc) = self.documents.get(path) {
            let text = doc.editor.read(cx).buffer().rope().clone();
            services.did_save(&doc.id, &text);
        }
        services.files_changed(&[FileChange::new(path, FileChangeKind::Changed)]);
    }

    /// The application changed files on disk (outside the editors).
    pub fn files_changed(&mut self, changes: &[FileChange], _: &mut Context<Self>) {
        if let Some(services) = &self.services
            && !changes.is_empty()
        {
            services.files_changed(changes);
        }
    }

    /// Another tab became the active one (`None`: no tab).
    pub fn active_file_changed(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        if self.active.as_deref() == path {
            return;
        }
        self.active = path.map(Path::to_owned);
        self.close_completion(cx);
        self.close_doc(cx);
        self.reset_signature(cx);
        self.reset_navigation(cx);
        self.clear_bulb(cx);
        self.popups.clear();
        self.refresh_services_popup(cx);
        cx.notify();
    }

    fn editor_event(&mut self, path: &Path, event: &EditorEvent, cx: &mut Context<Self>) {
        if let Some(editor) = self.documents.get(path).map(|d| d.editor.clone()) {
            self.doc_editor_event(&editor, event, cx);
            self.signature_editor_event(&editor, event, cx);
            self.nav_editor_event(&editor, event, cx);
            self.intention_editor_event(&editor, event, cx);
            self.occurrence_editor_event(&editor, event, cx);
        }
        match event {
            EditorEvent::Edited(applied) => {
                let Some(doc) = self.documents.get_mut(path) else {
                    return;
                };
                let version = doc
                    .versions
                    .edited(applied, doc.editor.read(cx).buffer().rope());
                if let Some(services) = &self.services {
                    services.did_change(&doc.id, version, applied, doc.versions.text());
                }
                doc.reports = 0;
                let editor = doc.editor.entity_id();
                self.update_analysis(path, cx);
                if self.popups.hide(HINT_CONTEXT) {
                    cx.notify();
                }
                // A change before the completed identifier ends the popup.
                if let Some(session) = &self.completion
                    && session.path == path
                    && applied.iter().any(|e| e.start < session.start)
                {
                    self.close_completion(cx);
                }
                self.typed(path, editor, applied, cx);
            }
            EditorEvent::Reloaded => {
                let Some(doc) = self.documents.get_mut(path) else {
                    return;
                };
                let text = doc.editor.read(cx).buffer().rope().clone();
                let version = doc.versions.reloaded(&text);
                if let Some(services) = &self.services {
                    services.did_reload(&doc.id, version, &text);
                }
                doc.reports = 0;
                let editor = doc.editor.entity_id();
                self.update_analysis(path, cx);
                if self.completion.as_ref().is_some_and(|s| s.path == path) {
                    self.close_completion(cx);
                }
                self.popups.hide_editor(editor);
                cx.notify();
            }
            EditorEvent::SelectionsChanged
                if self.completion.as_ref().is_some_and(|s| s.path == path) =>
            {
                self.completion_moved(cx);
            }
            _ => {}
        }
    }

    fn editor_blurred(&mut self, editor: EntityId, cx: &mut Context<Self>) {
        self.bulb_focus_changed(editor, cx);
        self.signature_blurred(editor, cx);
        self.doc_blurred(editor, cx);
        if self
            .completion
            .as_ref()
            .is_some_and(|s| s.editor.entity_id() == editor)
        {
            self.close_completion(cx);
        }
        self.popups.hide(HINT_CONTEXT);
    }

    fn service_event(&mut self, event: IntelEvent, cx: &mut Context<Self>) {
        self.server_tasks.event(&event, self.root.as_deref());
        match event {
            IntelEvent::Diagnostics {
                doc,
                version,
                items,
            } => {
                let Some(document) = self.documents.get_mut(doc.path()) else {
                    return;
                };
                // A report for a text that is not known any more: a newer
                // one follows.
                let Some(marks) = marks::marks(&document.versions, version, &items) else {
                    return;
                };
                if version == document.version() {
                    document.reports += 1;
                }
                document
                    .editor
                    .update(cx, |e, cx| e.set_diagnostics(marks, cx));
                self.update_analysis(doc.path(), cx);
                self.diagnostics_reported(&doc, version, &items);
            }
            IntelEvent::Status(_) => {
                // Servers starting, stopping or taking documents change
                // whose analysis is due.
                let paths: Vec<PathBuf> = self.documents.keys().cloned().collect();
                for path in paths {
                    self.update_analysis(&path, cx);
                }
                self.refresh_services_popup(cx);
                cx.notify();
            }
            _ => {}
        }
    }

    // ---- analysis ---------------------------------------------------------------

    /// How far the analysis of a document is, for the editor's inspection
    /// widget: running while a server that reports diagnostics on it (one
    /// that runs or starts) has not reported on its current text, done
    /// when all have; off when none does (nothing analyses the file, as
    /// for files of other types).
    fn update_analysis(&mut self, path: &Path, cx: &mut Context<Self>) {
        let (Some(doc), Some(services)) = (self.documents.get(path), &self.services) else {
            return;
        };
        let reporting = services
            .status()
            .iter()
            .filter(|s| {
                s.serves(&doc.id)
                    && s.features.contains(Feature::Diagnostics)
                    && s.state.is_running()
            })
            .count();
        let analysis = if reporting == 0 {
            Analysis::Off
        } else if doc.reports >= reporting {
            Analysis::Done
        } else {
            Analysis::Running
        };
        doc.editor.update(cx, |e, cx| e.set_analysis(analysis, cx));
    }

    // ---- completion -----------------------------------------------------------

    /// Basic Completion in an editor: the popup for the identifier at the
    /// caret, filled when the services answer.
    pub fn complete(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        if !editor.read(cx).selections().primary().is_empty() {
            return;
        }
        self.popups.hide(HINT_CONTEXT);
        let start = identifier_start(editor.read(cx).buffer(), query.offset);
        self.start_completion(editor, query.path, start, true, cx);
        self.request_completion(CompletionTrigger::Invoked, cx);
    }

    /// A char was typed in a document's editor: an identifier char or a
    /// trigger char of its servers opens the popup by itself (the
    /// reference's `CompletionAutoPopupHandler` and
    /// `LspAutoPopupTypedHandler`), except in strings and comments and in
    /// numbers.
    fn typed(
        &mut self,
        path: &Path,
        editor_id: EntityId,
        applied: &[AppliedEdit],
        cx: &mut Context<Self>,
    ) {
        let Some(c) = typed_char(applied) else {
            self.empty_auto = None;
            return;
        };
        let (Some(doc), Some(services)) = (self.documents.get(path), self.services.clone()) else {
            return;
        };
        let editor = doc.editor.clone();
        let e = editor.read(cx);
        let selections = e.selections();
        let caret = selections.primary().head;
        // Undo and redo put text back too; a typed char clears the redo
        // history.
        if e.buffer().can_redo()
            || selections.len() != 1
            || !selections.primary().is_empty()
            || caret != applied[0].new_end
        {
            return;
        }
        let identifier = is_identifier_char(c);
        let trigger = !identifier && services.completion_triggers(&doc.id).contains(&c);
        if !identifier {
            self.empty_auto = None;
        }
        // An open popup follows the identifier typed in it; another char
        // ends it.
        if let Some(session) = &self.completion
            && session.editor.entity_id() == editor_id
        {
            if identifier && caret > session.start {
                return;
            }
            self.close_completion(cx);
        }
        if !identifier && !trigger {
            return;
        }
        let start = if identifier {
            identifier_start(editor.read(cx).buffer(), caret)
        } else {
            caret
        };
        if identifier
            && (editor
                .read(cx)
                .buffer()
                .char_at(start)
                .is_some_and(|c| c.is_ascii_digit())
                || self.empty_auto == Some((editor_id, start)))
        {
            return;
        }
        if in_string_or_comment(&editor, caret - 1, cx) {
            return;
        }
        self.start_completion(&editor, path.to_owned(), start, false, cx);
        let trigger = if trigger {
            CompletionTrigger::Character(c)
        } else {
            CompletionTrigger::Invoked
        };
        self.request_completion(trigger, cx);
    }

    fn start_completion(
        &mut self,
        editor: &Entity<Editor>,
        path: PathBuf,
        start: usize,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        self.close_completion(cx);
        self.sessions += 1;
        let mut session = Session::new(self.sessions, path, editor.clone(), start, explicit);
        session.member = start > 0 && editor.read(cx).buffer().char_at(start - 1) == Some('.');
        self.completion = Some(session);
    }

    /// Asks the services for the items at the caret of the session's
    /// editor; the request in flight, if any, is cancelled.
    fn request_completion(&mut self, trigger: CompletionTrigger, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let Some(session) = &self.completion else {
            return;
        };
        let editor = session.editor.clone();
        let Some(query) = self.caret_query(&editor, cx) else {
            return;
        };
        let Some(session) = self.completion.as_mut() else {
            return;
        };
        let buffer = editor.read(cx).buffer();
        session.requested_prefix = buffer.slice(session.start..query.offset.max(session.start));
        let future = services.completion(&query.doc, query.at, trigger);
        let (id, version) = (session.id, query.version);
        session.request = Some(cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                this.completion_answered(id, version, result, cx)
            });
        }));
        cx.notify();
    }

    fn completion_answered(
        &mut self,
        id: u64,
        version: u64,
        result: Result<Completions, IntelError>,
        cx: &mut Context<Self>,
    ) {
        // An answer for a popup that is gone.
        let Some(session) = self.completion.as_mut().filter(|s| s.id == id) else {
            return;
        };
        session.request = None;
        let Ok(answer) = result else {
            self.close_completion(cx);
            return;
        };
        session.items = answer.items;
        session.incomplete = answer.incomplete;
        session.answer_version = version;
        session.answered = true;
        self.completion_filter(cx);
        cx.emit(CodeIntelEvent::Redraw);
    }

    /// The identifier typed so far in the session's editor, `None` when
    /// the caret left it (or a selection or another caret appeared).
    fn completion_prefix(&self, cx: &App) -> Option<String> {
        let session = self.completion.as_ref()?;
        let editor = session.editor.read(cx);
        let selections = editor.selections();
        let caret = selections.primary().head;
        if selections.len() != 1 || !selections.primary().is_empty() || caret < session.start {
            return None;
        }
        Some(editor.buffer().slice(session.start..caret)).filter(|p| {
            p.chars()
                .all(|c| is_identifier_char(c) || session.through.contains(&c))
        })
    }

    /// The caret or the text of the session's editor changed: the popup
    /// follows the identifier typed, or closes when the caret leaves it.
    fn completion_moved(&mut self, cx: &mut Context<Self>) {
        let Some(prefix) = self.completion_prefix(cx) else {
            self.close_completion(cx);
            return;
        };
        if self
            .completion
            .as_ref()
            .is_some_and(|s| s.answered && s.prefix != prefix)
        {
            self.completion_filter(cx);
        }
    }

    /// Filters the items with the typed prefix and shows them; an answer
    /// the server marked incomplete is asked for again with the new prefix.
    fn completion_filter(&mut self, cx: &mut Context<Self>) {
        let Some(prefix) = self.completion_prefix(cx) else {
            self.close_completion(cx);
            return;
        };
        let Some(session) = self.completion.as_mut() else {
            return;
        };
        session.refilter(&prefix);
        if session.incomplete && prefix != session.requested_prefix {
            self.request_completion(CompletionTrigger::Incomplete, cx);
        }
        self.show_completion(cx);
    }

    /// Shows the session's items. Without any (and nothing more coming)
    /// the popup closes: Basic Completion says so in a hint, an automatic
    /// popup opens no more while the identifier is typed on. An automatic
    /// popup whose items are all typed out already closes too.
    fn show_completion(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.completion else {
            return;
        };
        let pending = session.incomplete || session.request.is_some();
        let meaningless = !session.explicit
            && !session.focused
            && !session.shown.is_empty()
            && session
                .shown
                .iter()
                .all(|s| session.items[s.item].insert.text() == session.prefix);
        if session.shown.is_empty() && pending {
            self.popups.hide(completion::CONTEXT);
            cx.notify();
            return;
        }
        if session.shown.is_empty() || meaningless {
            let (editor, start, explicit) =
                (session.editor.clone(), session.start, session.explicit);
            self.close_completion(cx);
            if explicit {
                self.show_hint(&editor, start, NO_SUGGESTIONS, cx);
            } else {
                self.empty_auto = Some((editor.entity_id(), start));
            }
            return;
        }
        let editor = session.editor.clone();
        let rows = session.shown.len();
        self.completion_list
            .update(cx, |list, cx| list.set(session, cx));
        self.popups.show(CaretPopup {
            context: completion::CONTEXT,
            editor,
            anchor: session.start,
            placement: Placement::Below,
            inset: CompletionList::text_inset(),
            height: Some(CompletionList::height(rows, cx)),
            content: self.completion_list.clone().into(),
        });
        cx.notify();
    }

    /// A one-line hint at a position of an editor (closed by the next key,
    /// edit or caret move).
    pub(crate) fn show_hint(
        &mut self,
        editor: &Entity<Editor>,
        anchor: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let text = SharedString::from(text.to_owned());
        let content = cx.new(|_| TextHint::new(text));
        self.popups.show(CaretPopup {
            context: HINT_CONTEXT,
            editor: editor.clone(),
            anchor,
            placement: Placement::Below,
            inset: gpui::px(0.0),
            height: None,
            content: content.into(),
        });
        cx.emit(CodeIntelEvent::Redraw);
        cx.notify();
    }

    /// Closes the completion popup; the request in flight is cancelled.
    fn close_completion(&mut self, cx: &mut Context<Self>) {
        let had_session = self.completion.take().is_some();
        let had_popup = self.popups.hide(completion::CONTEXT);
        if had_session || had_popup {
            cx.notify();
        }
    }

    fn completion_shown(&self) -> bool {
        self.popups.is_shown(completion::CONTEXT)
    }

    /// Moves the selection; it also focuses an automatic popup.
    fn select_completion(&mut self, f: impl FnOnce(&mut Session), cx: &mut Context<Self>) {
        if let Some(session) = self.completion.as_mut() {
            session.focused = true;
            f(session);
            self.show_completion(cx);
        }
    }

    /// Inserts the selected item: in place of the typed prefix, or of the
    /// whole identifier at the caret (`replace`), with the item's other
    /// edits, as one undo step.
    fn accept_completion(&mut self, replace: bool, cx: &mut Context<Self>) {
        if !self.completion_shown() {
            return;
        }
        let Some(session) = self.completion.take() else {
            return;
        };
        self.popups.hide(completion::CONTEXT);
        cx.notify();
        let (Some(index), Some(doc)) = (session.selected_item(), self.documents.get(&session.path))
        else {
            return;
        };
        let item = &session.items[index];
        let parentheses = self.hook_for(&doc.language).0.parentheses(&doc.language);
        session.editor.update(cx, |e, cx| {
            let caret = e.selections().primary().head;
            let (edits, caret) = completion::insertion(
                item,
                &doc.versions,
                session.answer_version,
                e.buffer(),
                session.start,
                caret,
                replace,
                parentheses,
            );
            e.edit(
                edits,
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::caret(caret)),
                cx,
            );
        });
    }

    /// A character is about to be typed in the editor of a document (the
    /// editor's type hook; the reference's `LookupTypedHandler`). When the
    /// popup's selection is the person's and the character is a completion
    /// character, the selected item goes in first and the character is
    /// typed after it, unless the insertion took it over (a function's
    /// `(`). Returns the edit the hook makes in the editor (the editor is
    /// the one being updated: it is read here, never reached through its
    /// entity), `None` to type the character as usual.
    fn completion_char(
        &mut self,
        path: &Path,
        editor: &Editor,
        c: char,
        cx: &mut Context<Self>,
    ) -> Option<CharInsertion> {
        let session = self.completion.as_ref().filter(|s| s.path == path)?;
        let selections = editor.selections();
        let caret = selections.primary().head;
        if !self.completion_shown()
            || !session.inserts_on(c)
            || selections.len() != 1
            || !selections.primary().is_empty()
            || caret < session.start
            // The popup follows what is typed; an older one is not trusted.
            || editor.buffer().slice(session.start..caret) != session.prefix
        {
            return None;
        }
        if let Some(through) = session.prefix_through(c) {
            // The popup stays open and follows the longer prefix.
            let start = session.start;
            if let Some(session) = self.completion.as_mut() {
                session.through.push(c);
            }
            return Some(CharInsertion {
                caret: start + through.chars().count(),
                edits: vec![Edit::new(start..caret, through)],
                kind: EditKind::Typing,
                consumed: true,
            });
        }
        let index = session.selected_item()?;
        let doc = self.documents.get(path)?;
        let parentheses = self.hook_for(&doc.language).0.parentheses(&doc.language);
        let session = self.completion.take()?;
        self.popups.hide(completion::CONTEXT);
        cx.notify();
        let (edits, caret, consumed) = completion::insertion_by_char(
            &session.items[index],
            &doc.versions,
            session.answer_version,
            editor.buffer(),
            session.start,
            caret,
            parentheses,
            c,
        );
        Some(CharInsertion {
            edits,
            kind: EditKind::Other,
            caret,
            consumed,
        })
    }

    /// A click on a row of the completion popup inserts its item.
    pub(crate) fn click_completion_row(
        &mut self,
        index: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.completion.as_mut() {
            session.select(index);
        }
        self.accept_completion(false, cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select_completion(|s| s.select_by(1, true), cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.select_completion(|s| s.select_by(-1, true), cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let page = completion::list::VISIBLE_ROWS as isize;
        self.select_completion(|s| s.select_by(page, false), cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = completion::list::VISIBLE_ROWS as isize;
        self.select_completion(|s| s.select_by(-page, false), cx);
    }

    fn first(&mut self, _: &First, _: &mut Window, cx: &mut Context<Self>) {
        self.select_completion(|s| s.select(0), cx);
    }

    fn last(&mut self, _: &Last, _: &mut Window, cx: &mut Context<Self>) {
        self.select_completion(|s| s.select(s.shown.len().saturating_sub(1)), cx);
    }

    fn accept(&mut self, _: &Accept, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_completion(false, cx);
    }

    fn replace(&mut self, _: &Replace, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_completion(true, cx);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.close_completion(cx);
    }

    // ---- Language Services ------------------------------------------------------

    /// The servers of the project as the widget shows them, errors first.
    pub fn server_items(&self, cx: &App) -> Vec<ServerItem> {
        let Some(services) = &self.services else {
            return Vec::new();
        };
        let current = self.active_document().map(|d| d.id.clone());
        let mut items: Vec<ServerItem> = services
            .status()
            .iter()
            .map(|status| {
                let provider = &self.hook_of(status).0;
                ServerItem::new(
                    status,
                    provider.server_title(status),
                    provider.server_icon(status),
                    provider.server_status_bar_icon(status),
                    current.as_ref().is_some_and(|doc| status.serves(doc)),
                )
            })
            .collect();
        let _ = cx;
        items.sort_by_key(|i| !i.error);
        items
    }

    pub(crate) fn widget_bounds(&self) -> WidgetBounds {
        self.widget_bounds.clone()
    }

    /// Opens the Language Services popup over the widget, or closes it.
    pub fn toggle_services_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.services_popup.take().is_some() {
            cx.notify();
            return;
        }
        let items = self.server_items(cx);
        if items.is_empty() {
            return;
        }
        let popup = ServicesPopup::open(cx.weak_entity(), items, window, cx);
        let subscription = cx.subscribe(&popup, |this, _, _: &DismissEvent, cx| {
            this.services_popup = None;
            cx.notify();
        });
        self.services_popup = Some((popup, subscription));
        cx.notify();
    }

    fn refresh_services_popup(&mut self, cx: &mut Context<Self>) {
        let items = self.server_items(cx);
        if let Some((popup, _)) = &self.services_popup {
            popup.update(cx, |p, cx| {
                p.set_items(items);
                cx.notify();
            });
        }
    }

    /// An action of a server's row in the popup.
    pub(crate) fn run_server_action(
        &mut self,
        item: &ServerItem,
        action: InlineAction,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            InlineAction::Restart => self.restart_server(Some(&item.id), cx),
            InlineAction::Stop => self.stop_server_by_id(&item.id, cx),
            InlineAction::ErrorOutput => {
                if let Some(text) = &item.error_output {
                    cx.emit(CodeIntelEvent::OpenText {
                        title: format!("{} Server Error Output", item.title),
                        text: text.clone(),
                    });
                }
            }
            InlineAction::Settings => self.open_settings(cx),
        }
    }

    /// Restarts a server, or all; a notice names each (the reference's
    /// balloon "Ruff server restarted").
    fn restart_server(&mut self, server: Option<&ServerId>, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let titles: Vec<String> = self
            .server_items(cx)
            .into_iter()
            .filter(|i| server.is_none_or(|s| *s == i.id))
            .map(|i| i.title)
            .collect();
        services.restart(server);
        for title in titles {
            cx.emit(CodeIntelEvent::Notice(format!("{title} server restarted")));
        }
    }

    /// Stops a server; it starts again when a document needs it.
    fn stop_server_by_id(&mut self, server: &ServerId, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let title = self
            .server_items(cx)
            .into_iter()
            .find(|i| i.id == *server)
            .map(|i| i.title);
        services.stop(server);
        if let Some(title) = title {
            cx.emit(CodeIntelEvent::Notice(format!(
                "{title} server stopped. The server will restart automatically when needed"
            )));
        }
    }

    /// The settings of the language services: the project's settings
    /// file, until there is a settings window.
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let Some(root) = &self.root else {
            return;
        };
        match self.hooks[0].0.settings_file(root) {
            Some(file) if file.is_file() => cx.emit(CodeIntelEvent::OpenFile(file)),
            Some(file) => cx.emit(CodeIntelEvent::Notice(format!(
                "The project has no settings file yet ({})",
                file.strip_prefix(root).unwrap_or(&file).display()
            ))),
            None => {}
        }
    }

    // ---- drawing and state ------------------------------------------------------

    /// The popups over the editor (completion, documentation, parameter
    /// info) and the Language Services popup, drawn by the workbench above
    /// its content.
    pub fn render_overlay(&self, window: &Window, cx: &App) -> Vec<AnyElement> {
        let mut overlay = self.popups.render(window, cx);
        overlay.extend(self.render_bulb(window, cx));
        if let Some((popup, _)) = &self.services_popup
            && !popup.read(cx).is_dismissed()
        {
            overlay.push(services::overlay(popup, self.widget_bounds.get(), window));
        }
        overlay
    }

    /// The state reported by `code.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let mut documents: Vec<Json> = self
            .documents
            .values()
            .map(|d| {
                json!({
                    "path": d.id.path().display().to_string(),
                    "language": d.language,
                    "version": d.version(),
                })
            })
            .collect();
        documents.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        // Every message of a mark is a problem (the reference counts per
        // problem, not per range).
        let problems = self.active_document().map(|d| {
            d.editor
                .read(cx)
                .diagnostics()
                .iter()
                .flat_map(|m| &m.messages)
                .filter(|m| m.severity != editor::MarkSeverity::Information)
                .count()
        });
        let items = self.server_items(cx);
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "services": self.services.is_some(),
            "documents": documents,
            "active": self.active.as_ref().map(|p| p.display().to_string()),
            "popups": self.popups.contexts().collect::<Vec<_>>(),
            "completion": self.completion.as_ref().map(|s| {
                let mut state = s.state_json(self.completion_shown());
                state["hint"] = completion::list::hint(cx).into();
                state
            }),
            "hint": self
                .popups
                .get(HINT_CONTEXT)
                .and_then(|p| p.content.clone().downcast::<TextHint>().ok())
                .map(|h| h.read(cx).text().to_string()),
            "problems": problems,
            "widget": services::WidgetState::new(&items).map(|w| w.state_json()),
            "services_popup": self.services_popup.as_ref().map(|(p, _)| p.read(cx).state_json()),
            "popup": self.doc_state_json(cx),
            "tool_window": self.doc_window().read(cx).state_json(cx),
            "signature": self.signature_state_json(cx),
            "list": self.list_state_json(cx),
            "bulb": self.bulb_state_json(),
        })
    }

    /// The servers reported by `code.servers`.
    pub fn servers_json(&self, cx: &App) -> Json {
        let _ = cx;
        let servers = self
            .services
            .as_ref()
            .map(|s| s.status())
            .unwrap_or_default();
        serde_json::to_value(servers).unwrap_or(Json::Null)
    }

    // ---- commands ----------------------------------------------------------

    fn active_editor(&self) -> Option<Entity<Editor>> {
        self.active_document().map(|d| d.editor.clone())
    }

    fn quick_doc(&mut self, _: &QuickDoc, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.quick_doc_at_caret(&editor, cx);
        }
    }

    fn parameter_info(&mut self, _: &ParameterInfo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.parameter_info_at_caret(&editor, cx);
        }
    }

    fn apply_first_fix(&mut self, _: &ApplyFirstFix, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.apply_first_fix_at_caret(&editor, cx);
        }
    }

    fn close_documentation(
        &mut self,
        _: &CloseDocumentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_doc(cx);
    }

    fn close_parameter_info(
        &mut self,
        _: &CloseParameterInfo,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_signature(cx);
    }

    fn go_to_declaration(&mut self, _: &GoToDeclaration, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.go_to_declaration_at_caret(&editor, cx);
        }
    }

    fn show_usages(&mut self, _: &ShowUsages, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.show_usages_at_caret(&editor, cx);
        }
    }

    fn rename_symbol(&mut self, _: &RenameSymbol, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.rename_at_caret(&editor, cx);
        }
    }

    fn file_structure(&mut self, _: &FileStructure, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.show_structure(&editor, cx);
        }
    }

    fn show_intentions(&mut self, _: &ShowIntentions, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.show_intentions_at_caret(&editor, cx);
        }
    }

    fn reformat(&mut self, _: &Reformat, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.reformat_editor(&editor, cx);
        }
    }

    fn optimize_imports(&mut self, _: &OptimizeImports, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.optimize_imports_in(&editor, cx);
        }
    }

    fn error_description(&mut self, _: &ErrorDescription, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.error_description_at_caret(&editor, cx);
        }
    }

    fn restart_servers(&mut self, action: &RestartServers, _: &mut Window, cx: &mut Context<Self>) {
        let server = action.server.as_deref().map(ServerId::new);
        self.restart_server(server.as_ref(), cx);
    }

    fn stop_server(&mut self, action: &StopServer, _: &mut Window, cx: &mut Context<Self>) {
        self.stop_server_by_id(&ServerId::new(&action.server), cx);
    }
}

/// The char typed by an edit: one char inserted without replacing
/// anything (`None` for anything else, such as a paste or a deletion).
/// What the popup does to the editor when a completion character is typed:
/// the edits, the caret after them and whether they took the character over.
struct CharInsertion {
    edits: Vec<Edit>,
    kind: EditKind,
    caret: usize,
    consumed: bool,
}

fn typed_char(applied: &[AppliedEdit]) -> Option<char> {
    let [edit] = applied else {
        return None;
    };
    let mut chars = edit.text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if edit.start == edit.old_end => Some(c),
        _ => None,
    }
}

/// The char at an offset is in a string or a comment, as the editor's
/// highlighting (the language's lexer) colours it.
fn in_string_or_comment(editor: &Entity<Editor>, offset: usize, cx: &mut App) -> bool {
    editor.update(cx, |e, _| {
        let point = e.buffer().offset_to_point(offset);
        e.highlights(point.line).iter().any(|(range, role)| {
            range.contains(&point.column)
                && matches!(
                    role,
                    TextRole::String
                        | TextRole::StringEscape
                        | TextRole::StringEscapeInvalid
                        | TextRole::Comment
                        | TextRole::Docstring
                        | TextRole::DocstringTag
                        | TextRole::Todo
                )
        })
    })
}

/// Adds handlers to `el` (the workbench root) that send the code
/// intelligence commands that do not depend on the focused editor to
/// `view`, wherever the focus is in the window.
pub fn forward_actions<E: InteractiveElement>(el: E, view: Entity<CodeIntelView>) -> E {
    macro_rules! forward {
        ($el:expr, $($action:ty => $method:ident),* $(,)?) => {
            $el $(.on_action({
                let view = view.clone();
                move |a: &$action, window, cx| view.update(cx, |v, cx| v.$method(a, window, cx))
            }))*
        };
    }
    forward!(el,
        QuickDoc => quick_doc,
        OpenInToolWindow => open_in_tool_window,
        ToggleShowPopupFirst => toggle_show_popup_first,
        ToggleShowOnMouseMove => toggle_show_on_mouse_move,
        ParameterInfo => parameter_info,
        GoToDeclaration => go_to_declaration,
        ShowUsages => show_usages,
        ShowIntentions => show_intentions,
        RenameSymbol => rename_symbol,
        FileStructure => file_structure,
        Reformat => reformat,
        OptimizeImports => optimize_imports,
        ErrorDescription => error_description,
        ApplyFirstFix => apply_first_fix,
        RestartServers => restart_servers,
        StopServer => stop_server,
    )
}

/// The container of a tab's editor as the workbench draws it, with what
/// code intelligence adds around a document's editor: the commands that
/// act at its caret (Basic Completion) and, while a popup shows for it,
/// the popup's key context and keys. Other elements come back unchanged.
pub fn editor_container<E: InteractiveElement>(
    el: E,
    view: &Entity<CodeIntelView>,
    editor: &Entity<Editor>,
    cx: &App,
) -> E {
    let v = view.read(cx);
    if v.document_of(editor).is_none() {
        return el;
    }
    let mut el = el.on_action({
        let (view, editor) = (view.clone(), editor.clone());
        move |_: &Complete, _, cx| view.update(cx, |v, cx| v.complete(&editor, cx))
    });
    if let Some(context) = v.popups.key_context(editor.entity_id()) {
        el = el.key_context(context.as_str());
    }
    let completion = v
        .popups
        .get(completion::CONTEXT)
        .is_some_and(|p| p.editor.entity_id() == editor.entity_id());
    // The popups of documentation and parameter info close with Escape.
    let shown = |context| {
        v.popups
            .get(context)
            .is_some_and(|p| p.editor.entity_id() == editor.entity_id())
    };
    if shown(doc::CONTEXT) {
        el = el
            .on_action({
                let view = view.clone();
                move |a: &CloseDocumentation, window, cx| {
                    view.update(cx, |v, cx| v.close_documentation(a, window, cx))
                }
            })
            .on_action({
                let view = view.clone();
                move |_: &EditDocumentationSource, _, cx| {
                    view.update(cx, |v, cx| v.edit_documentation_source(cx))
                }
            })
            .on_action({
                let view = view.clone();
                move |_: &ViewExternalDocumentation, _, cx| {
                    view.update(cx, |v, cx| v.view_external_documentation(cx))
                }
            });
        el = crate::hover::doc_popup_actions(el, view);
    }
    if shown(signature::CONTEXT) {
        el = el.on_action({
            let view = view.clone();
            move |a: &CloseParameterInfo, window, cx| {
                view.update(cx, |v, cx| v.close_parameter_info(a, window, cx))
            }
        });
    }
    if completion {
        macro_rules! bind {
            ($el:expr, $($action:ty => $method:ident),* $(,)?) => {
                $el $(.on_action({
                    let view = view.clone();
                    move |a: &$action, window, cx| view.update(cx, |v, cx| v.$method(a, window, cx))
                }))*
            };
        }
        el = bind!(el,
            SelectNext => select_next,
            SelectPrevious => select_previous,
            PageDown => page_down,
            PageUp => page_up,
            First => first,
            Last => last,
            Accept => accept,
            Replace => replace,
            Cancel => cancel,
        );
    }
    el
}

#[cfg(test)]
mod tests {
    use code_intel::fake::{Call, FakeCodeIntel};
    use gpui::TestAppContext;

    use super::*;
    use crate::CodeIntelProvider;

    struct Fake(Arc<FakeCodeIntel>);

    impl CodeIntelProvider for Fake {
        fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
            self.0.clone()
        }
    }

    #[gpui::test]
    fn a_project_gets_its_services(cx: &mut TestAppContext) {
        let fake = FakeCodeIntel::new();
        let hook = CodeIntelHook::new(Fake(fake.clone()));
        let view = cx.new(|cx| CodeIntelView::new(vec![hook], cx));
        view.update(cx, |v, cx| {
            assert!(v.services().is_none());
            v.project_opened(Path::new("/p"), cx);
            assert_eq!(v.project_root(), Some(Path::new("/p")));
            assert!(v.services().is_some());
            assert_eq!(v.state_json(cx)["services"], true);
            assert_eq!(v.servers_json(cx), json!([]));
        });
        // Another project: the previous services shut down.
        view.update(cx, |v, cx| v.project_opened(Path::new("/q"), cx));
        cx.run_until_parked();
        assert_eq!(fake.calls(), [Call::Shutdown]);
    }

    /// The services of some languages, with the tool's name in its
    /// servers' titles.
    struct Languages(Arc<FakeCodeIntel>, &'static [&'static str], &'static str);

    impl CodeIntelProvider for Languages {
        fn languages(&self) -> &[&'static str] {
            self.1
        }

        fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
            self.0.clone()
        }

        fn server_title(&self, server: &ServerStatus) -> String {
            format!("{} {}", self.2, server.name)
        }
    }

    #[gpui::test]
    fn several_providers_answer_for_their_languages(cx: &mut TestAppContext) {
        let python = FakeCodeIntel::new();
        let sql = FakeCodeIntel::new();
        python.set_status(vec![ServerStatus::new(ServerId::new("ty"), "ty")]);
        sql.set_status(vec![ServerStatus::new(ServerId::new("sql"), "sql")]);
        let hooks = vec![
            CodeIntelHook::new(Languages(python.clone(), &[], "Python")),
            CodeIntelHook::new(Languages(sql.clone(), &["sql"], "SQL")),
        ];
        let view = cx.new(|cx| CodeIntelView::new(hooks, cx));
        view.update(cx, |v, cx| {
            assert_eq!(v.hook_for("sql").languages(), ["sql"]);
            assert!(v.hook_for("python").languages().is_empty());
            v.project_opened(Path::new("/p"), cx);
            // Each server is named by its own provider.
            let mut titles: Vec<String> = v.server_items(cx).into_iter().map(|i| i.title).collect();
            titles.sort();
            assert_eq!(titles, ["Python ty", "SQL sql"]);
        });
        view.update(cx, |v, cx| v.project_opened(Path::new("/q"), cx));
        cx.run_until_parked();
        assert_eq!(python.calls(), [Call::Shutdown]);
        assert_eq!(sql.calls(), [Call::Shutdown]);
    }

    #[test]
    fn typed_chars() {
        let edit = |start, old_end, text: &str| AppliedEdit {
            start,
            old_end,
            new_end: start + text.chars().count(),
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: start + text.len(),
            start_position: (0, start),
            old_end_position: (0, old_end),
            new_end_position: (0, start + text.len()),
            start_utf16_col: start,
            old_end_utf16_col: old_end,
            text: text.into(),
        };
        assert_eq!(typed_char(&[edit(3, 3, ".")]), Some('.'));
        assert_eq!(typed_char(&[edit(3, 3, "ab")]), None);
        assert_eq!(typed_char(&[edit(3, 4, "a")]), None);
        assert_eq!(typed_char(&[edit(3, 3, "a"), edit(5, 5, "a")]), None);
    }
}
