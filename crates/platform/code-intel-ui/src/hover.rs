//! When the documentation popup shows (`doc` draws it): Quick
//! Documentation at the caret; the popup over the text under the mouse
//! after the reference's tooltip delay of 500 ms
//! (`EditorSettingsExternalizable`), with the problems there, their first
//! quick fix and the symbol's documentation; Error Description at the
//! caret. The popup over the text stays while the mouse is over the text
//! it is about or inside the popup (where it can scroll), and closes a
//! moment after the mouse left both. Escape, typing, a caret move, a blur
//! or another tab close it.
//!
//! The options of the popup's more menu (`settings::DocumentationSettings`)
//! decide where Quick Documentation shows (the popup, or the Documentation
//! tool window at once) and whether the popup over the text shows
//! documentation or the problems only.

use std::ops::Range;
use std::time::Duration;

use code_intel::{ActionTrigger, CodeActionContext, Hover, Location, Point};
use editor::{Editor, EditorEvent};
use gpui::{
    App, AsyncApp, Context, Entity, EntityId, InteractiveElement, Modifiers, Task, WeakEntity,
    Window,
};
use settings::{DocumentationSettings, Section as _};

use crate::commands::{
    AdjustFontSize, DocumentationBack, DocumentationForward, OpenInToolWindow,
    ToggleShowOnMouseMove, ToggleShowPopupFirst,
};
use crate::completion::{identifier_end, identifier_start, is_identifier_char};
use crate::doc::{self, DocContent, DocKind, DocLexer, DocPopup, Documentation, FileFacts, Fix};
use crate::doc_window::DocToolWindow;
use crate::file_doc::FileInfo;
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
    /// The Documentation tool window's view.
    pub(crate) window: Entity<DocToolWindow>,
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
    /// The popup under the mouse is off (while a debugger shows values
    /// there, `EditorMouseHoverPopupControl.disablePopups`).
    hover_off: bool,
}

impl DocState {
    pub(crate) fn new(popup: Entity<DocPopup>, window: Entity<DocToolWindow>) -> Self {
        Self {
            popup,
            window,
            under_mouse: None,
            delay: None,
            request: None,
            shown: None,
            inside: false,
            closing: None,
            generation: 0,
            hover_off: false,
        }
    }
}

/// The stored quick documentation options.
pub fn doc_settings(cx: &App) -> DocumentationSettings {
    settings_ui::store::get(cx)
}

/// Stores the quick documentation options.
fn set_doc_settings(settings: DocumentationSettings, cx: &mut App) {
    settings_ui::store::set(cx, &settings);
}

/// The popup and the tool window of a window draw again when the options
/// change (the font size, the check marks), wherever they changed.
pub(crate) fn follow_doc_settings(owner: &WeakEntity<CodeIntelView>, cx: &mut App) {
    let owner = owner.clone();
    settings_ui::store::observe(cx, DocumentationSettings::KEY, move |cx| {
        // After the current update: the change may come from the view
        // itself (the menu's options).
        let owner = owner.clone();
        cx.defer(move |cx| redraw_docs(&owner, cx));
    });
}

fn redraw_docs(owner: &WeakEntity<CodeIntelView>, cx: &mut App) {
    let Some(view) = owner.upgrade() else {
        return;
    };
    let (popup, window) = {
        let docs = &view.read(cx).docs;
        (docs.popup.clone(), docs.window.clone())
    };
    popup.update(cx, |_, cx| cx.notify());
    window.update(cx, |_, cx| cx.notify());
    view.update(cx, |_, cx| cx.notify());
}

/// The commands of the documentation popup's more menu that act on the
/// popup shown for the editor of `el`: Adjust Font Size, Back, Forward.
pub(crate) fn doc_popup_actions<E: InteractiveElement>(el: E, view: &Entity<CodeIntelView>) -> E {
    let popup = |view: &Entity<CodeIntelView>, cx: &App| view.read(cx).docs.popup.clone();
    el.on_action({
        let view = view.clone();
        move |_: &AdjustFontSize, window, cx| {
            let popup = popup(&view, cx);
            popup.update(cx, |p, cx| p.open_font_size(window, cx));
        }
    })
    .on_action({
        let view = view.clone();
        move |_: &DocumentationBack, _, cx| {
            let popup = popup(&view, cx);
            popup.update(cx, |p, cx| p.back(cx));
        }
    })
    .on_action({
        let view = view.clone();
        move |_: &DocumentationForward, _, cx| {
            let popup = popup(&view, cx);
            popup.update(cx, |p, cx| p.forward(cx));
        }
    })
}

/// What a request for documentation answers: the documentation and where
/// the symbol is declared.
type DocAnswer = (Option<Hover>, Option<Location>);

/// The facts of the file an answer is about, when the provider of the
/// document's language says the symbol is a whole file (a module): its
/// answer, then the file's facts read off the UI thread.
async fn file_facts(
    this: &WeakEntity<CodeIntelView>,
    language: &str,
    (hover, location): &DocAnswer,
    cx: &mut AsyncApp,
) -> Option<FileFacts> {
    let (hover, location) = (hover.as_ref()?, location.as_ref()?);
    let (task, root) = this
        .update(cx, |v, cx| {
            let root = v.project_root().map(std::path::Path::to_owned);
            let hook = v.hook_for(language).0.clone();
            let task = hook.file_doc(&hover.contents, &location.path, root.as_deref(), cx)?;
            Some((task, root))
        })
        .ok()??;
    let doc = task.await?;
    let (path, file_type) = (location.path.clone(), doc.file_type.clone());
    let info = cx
        .background_executor()
        .spawn(async move { FileInfo::read(&path, root.as_deref(), &file_type) })
        .await;
    Some(FileFacts { doc, info })
}

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
        if self.docs.hover_off {
            return;
        }
        let editor = editor.clone();
        self.docs.delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let _ = this.update(cx, |v, cx| v.hover_due(&editor, offset, cx));
        }));
    }

    /// Turns the popup under the mouse off or on again
    /// (`EditorMouseHoverPopupControl.disablePopups`, `enablePopups`).
    pub fn set_hover_popups(&mut self, on: bool, cx: &mut Context<Self>) {
        self.docs.hover_off = !on;
        if !on {
            self.docs.delay = None;
            if self
                .docs
                .shown
                .as_ref()
                .is_some_and(|s| s.kind == DocKind::Hover)
            {
                self.close_doc(cx);
            }
        }
    }

    /// The popup at a char offset of an editor at once, as the mouse
    /// resting there shows it, also while it is off
    /// (`EditorMouseHoverPopupManager.showInfoTooltip`).
    pub fn show_hover_at(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        self.docs.under_mouse = Some((editor.entity_id(), offset));
        self.docs.closing = None;
        self.hover_due(editor, offset, cx);
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
                if !v.docs.inside && !v.docs.popup.read(cx).menu_open() {
                    v.close_doc(cx);
                }
            });
        }));
    }

    /// The editor of a document lost the focus: its popup closes, unless
    /// the mouse is inside it.
    pub(crate) fn doc_blurred(&mut self, editor: EntityId, cx: &mut Context<Self>) {
        let ours = self.docs.shown.as_ref().is_some_and(|s| s.editor == editor);
        if ours && !self.docs.inside && !self.docs.popup.read(cx).menu_open() {
            self.close_doc(cx);
        }
    }

    /// The more menu of the popup closed: the popup over the text closes
    /// a moment later unless the mouse is back inside it.
    pub(crate) fn doc_menu_closed(&mut self, cx: &mut Context<Self>) {
        let hover = self
            .docs
            .shown
            .as_ref()
            .is_some_and(|s| s.kind == DocKind::Hover);
        if hover && !self.docs.inside {
            self.close_doc_later(cx);
        }
    }

    /// The documentation shown, when there is some.
    fn shown_documentation(&self, cx: &App) -> Option<Documentation> {
        self.docs.shown.as_ref()?;
        self.docs
            .popup
            .read(cx)
            .content()
            .and_then(|c| c.documentation.clone())
    }

    /// Jump to Source of the popup: opens the declaration it is about and
    /// closes it (`DocumentationEditSourceAction`).
    pub(crate) fn edit_documentation_source(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .docs
            .shown
            .as_ref()
            .and_then(|s| self.editor_by_id(s.editor))
        else {
            return;
        };
        let Some(source) = self.shown_documentation(cx).and_then(|d| d.source) else {
            return;
        };
        self.close_doc(cx);
        self.navigate(&editor, source, cx);
    }

    /// View External Documentation of the popup: opens its link in the
    /// browser and closes it.
    pub(crate) fn view_external_documentation(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.shown_documentation(cx).and_then(|d| d.external) else {
            return;
        };
        self.close_doc(cx);
        cx.open_url(&link.url);
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
        if problems.is_empty() && !(on_identifier && doc_settings(cx).show_on_mouse_move) {
            return;
        }
        let Some(services) = self.services().cloned() else {
            return;
        };
        let at = document.versions.point(offset);
        // Show on Mouse Move off: the problems there only
        // (`HoverPopupContext.documentationHoverInfo`).
        let documentation = (on_identifier && doc_settings(cx).show_on_mouse_move)
            .then(|| self.request_documentation(&doc_id, at));
        let language = document.language.clone();
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
            let facts = file_facts(&this, &language, &documentation, cx).await;
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
                let documentation = v.documentation(&editor, version, documentation, facts);
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

    /// The documentation of an answer (with the facts of the file it is
    /// about, when it is about a whole file), and the range in the current
    /// text it is about when the service said.
    fn documentation(
        &self,
        editor: &Entity<Editor>,
        version: u64,
        (hover, location): DocAnswer,
        facts: Option<FileFacts>,
    ) -> Option<(Documentation, Option<Range<usize>>)> {
        let hover = hover?;
        let document = self.document_of(editor)?;
        let provider = self.hook_for(&document.language).0.as_ref();
        let header = location
            .as_ref()
            .and_then(|l| provider.doc_location(&l.path));
        let lexer = DocLexer {
            provider,
            language: &document.language,
        };
        let mut documentation = Documentation::new(&hover.contents, header, Some(&lexer))?;
        documentation.source = location;
        if let Some(facts) = facts {
            documentation.set_file(facts);
        }
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
    /// the symbol at the caret (or right before it), in the popup, or in
    /// the Documentation tool window while Show Documentation Popup First
    /// is off (`DocumentationManager.showDocumentation`, `skipPopup`).
    pub(crate) fn quick_doc_at_caret(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let to_window = !doc_settings(cx).show_popup_first;
        self.quick_doc_to(editor, to_window, cx);
    }

    /// Quick Documentation at the caret, in the tool window or the popup.
    fn quick_doc_to(&mut self, editor: &Entity<Editor>, to_window: bool, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let buffer = editor.read(cx).buffer();
        let offset = symbol_offset(buffer, query.offset);
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let at = document.versions.point(offset);
        let language = document.language.clone();
        let request = self.request_documentation(&query.doc, at);
        self.docs.generation += 1;
        let generation = self.docs.generation;
        let (editor, version) = (editor.clone(), query.version);
        self.docs.request = Some(cx.spawn(async move |this, cx| {
            let answer = request.await;
            let facts = file_facts(&this, &language, &answer, cx).await;
            let _ = this.update(cx, |v, cx| {
                if v.docs.generation != generation {
                    return;
                }
                v.docs.request = None;
                let Some((documentation, _)) = v.documentation(&editor, version, answer, facts)
                else {
                    return;
                };
                if to_window {
                    v.show_in_tool_window(documentation, cx);
                    return;
                }
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
        let popup = self.docs.popup.read(cx);
        let mut state = popup.content().map(DocContent::to_json).unwrap_or_default();
        if let Some(object) = state.as_object_mut() {
            object.insert("menu".into(), popup.menu_state(cx).into());
            object.insert("font_size".into(), popup.font_size_state(cx).into());
        }
        state
    }

    /// The Documentation tool window's view (the workbench registers it).
    pub fn doc_window(&self) -> &Entity<DocToolWindow> {
        &self.docs.window
    }

    /// Shows documentation in the Documentation tool window.
    fn show_in_tool_window(&mut self, documentation: Documentation, cx: &mut Context<Self>) {
        self.docs
            .window
            .update(cx, |w, cx| w.set(documentation, cx));
        cx.emit(crate::view::CodeIntelEvent::ShowDocumentation);
    }

    /// Open in Documentation Tool Window: the popup's documentation moves
    /// there and the popup closes (`OpenInToolwindowAction`). Its shortcut
    /// is Quick Documentation's: over a popup without documentation (Error
    /// Description) it is Quick Documentation. Without a popup the
    /// documentation at the caret of the active editor goes to the tool
    /// window.
    pub(crate) fn open_in_tool_window(
        &mut self,
        _: &OpenInToolWindow,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(documentation) = self.shown_documentation(cx) {
            self.close_doc(cx);
            self.show_in_tool_window(documentation, cx);
            return;
        }
        let shown = self.docs.shown.is_some();
        if let Some(editor) = self.active_document().map(|d| d.editor.clone()) {
            if shown {
                self.close_doc(cx);
                self.quick_doc_at_caret(&editor, cx);
            } else {
                self.quick_doc_to(&editor, true, cx);
            }
        }
    }

    /// Show Documentation Popup First, on or off (a stored option).
    pub(crate) fn toggle_show_popup_first(
        &mut self,
        _: &ToggleShowPopupFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut settings = doc_settings(cx);
        settings.show_popup_first = !settings.show_popup_first;
        set_doc_settings(settings, cx);
    }

    /// Show on Mouse Move, on or off (a stored option).
    pub(crate) fn toggle_show_on_mouse_move(
        &mut self,
        _: &ToggleShowOnMouseMove,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut settings = doc_settings(cx);
        settings.show_on_mouse_move = !settings.show_on_mouse_move;
        set_doc_settings(settings, cx);
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
