//! Parameter info: the parameters of the call at the caret, above the
//! call, one line per signature (overload) with the parameter names only
//! (`obj, *, skipkeys, ...`, as the reference shows them) and the current
//! parameter in bold. It follows the caret while it stays in the call,
//! opens with Parameter Info and 1000 ms after `(` is typed (the
//! reference's `CodeInsightSettings.PARAMETER_INFO_DELAY`), and closes
//! with Escape, when the caret leaves the call, on a blur or another tab.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use code_intel::{IntelError, SignatureContext, SignatureHelp, SignatureTrigger};
use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, StyledText, Task, TextRun, Window, div, font, px,
};
use serde_json::{Value as Json, json};
use text::Buffer;
use themes::UiRole;
use ui::ThemeExt;

use crate::popup::{CaretPopup, Placement};
use crate::view::CodeIntelView;

/// The key context of the parameter info popup.
pub const CONTEXT: &str = "ParameterInfo";

/// How long after `(` is typed parameter info opens on its own.
pub const AUTO_DELAY: Duration = Duration::from_millis(1000);

/// How far back the opening bracket of a call is looked for.
const CALL_SEARCH_LIMIT: usize = 10_000;

/// The text of a signature without parameters.
const NO_PARAMETERS: &str = "<no parameters>";

/// One line of the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    /// The byte range of the current parameter's name.
    pub current: Option<Range<usize>>,
    /// Drawn grey (a signature without parameters).
    pub disabled: bool,
}

/// The name a parameter is shown by: its label up to its annotation or
/// default value, without the stars of `*args` and `**kwargs`; a bare `*`
/// or `/` stays as it is.
pub fn parameter_name(label: &str) -> String {
    let mut depth = 0i32;
    let mut end = label.len();
    for (i, c) in label.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ':' | '=' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let name = label[..end].trim();
    let bare = name.trim_start_matches('*').trim();
    if bare.is_empty() { name } else { bare }.to_owned()
}

/// The lines of a help: one per signature, the parameter names separated
/// by commas, the current one marked.
pub fn lines(help: &SignatureHelp) -> Vec<Line> {
    help.signatures
        .iter()
        .enumerate()
        .map(|(i, signature)| {
            let names: Vec<String> = signature
                .parameters
                .iter()
                .filter_map(|p| signature.label.get(p.label.clone()))
                .map(parameter_name)
                .collect();
            if names.is_empty() {
                return Line {
                    text: NO_PARAMETERS.to_owned(),
                    current: None,
                    disabled: true,
                };
            }
            let active = help.active_parameter_of(i);
            let mut text = String::new();
            let mut current = None;
            for (n, name) in names.iter().enumerate() {
                if n > 0 {
                    text.push_str(", ");
                }
                if Some(n) == active {
                    current = Some(text.len()..text.len() + name.len());
                }
                text.push_str(name);
            }
            Line {
                text,
                current,
                disabled: false,
            }
        })
        .collect()
}

/// The opening bracket of the call that contains `offset`: the nearest `(`
/// before it that is not closed before it. Other brackets in between are
/// skipped (an argument that is a list).
pub fn call_start(buffer: &Buffer, offset: usize) -> Option<usize> {
    let mut depth = 0usize;
    let start = offset.saturating_sub(CALL_SEARCH_LIMIT);
    let mut at = offset.min(buffer.len_chars());
    while at > start {
        at -= 1;
        match buffer.char_at(at)? {
            ')' | ']' | '}' => depth += 1,
            '(' if depth == 0 => return Some(at),
            '(' | '[' | '{' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// A parameter info popup over a call.
struct Session {
    id: u64,
    path: PathBuf,
    editor: Entity<Editor>,
    /// The opening bracket of the call.
    anchor: usize,
    help: Option<SignatureHelp>,
    request: Option<Task<()>>,
}

/// The parameter info state of a window.
pub(crate) struct SignatureState {
    session: Option<Session>,
    sessions: u64,
    /// Opens the popup a while after `(` was typed.
    auto: Option<Task<()>>,
    popup: Entity<SignaturePopup>,
}

impl SignatureState {
    pub(crate) fn new(popup: Entity<SignaturePopup>) -> Self {
        Self {
            session: None,
            sessions: 0,
            auto: None,
            popup,
        }
    }
}

impl CodeIntelView {
    /// Parameter Info in an editor: the signatures of the call at its
    /// caret.
    pub(crate) fn parameter_info_at_caret(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        self.signature_request(editor, SignatureTrigger::Invoked, cx);
    }

    /// Asks for the signatures at the caret of an editor; the popup opens
    /// or follows when they arrive.
    fn signature_request(
        &mut self,
        editor: &Entity<Editor>,
        trigger: SignatureTrigger,
        cx: &mut Context<Self>,
    ) {
        let (Some(query), Some(services)) =
            (self.caret_query(editor, cx), self.services().cloned())
        else {
            return;
        };
        let anchor = call_start(editor.read(cx).buffer(), query.offset);
        let same = self
            .signatures
            .session
            .as_ref()
            .is_some_and(|s| s.editor.entity_id() == editor.entity_id());
        if !same {
            self.close_signature(cx);
            self.signatures.sessions += 1;
            self.signatures.session = Some(Session {
                id: self.signatures.sessions,
                path: query.path.clone(),
                editor: editor.clone(),
                anchor: anchor.unwrap_or(query.offset),
                help: None,
                request: None,
            });
        }
        let Some(session) = self.signatures.session.as_mut() else {
            return;
        };
        if let Some(anchor) = anchor {
            session.anchor = anchor;
        }
        let context = SignatureContext {
            trigger,
            active: session.help.clone(),
        };
        let request = services.signature_help(&query.doc, query.at, context);
        let id = session.id;
        session.request = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |v, cx| v.signature_answered(id, result, cx));
        }));
    }

    fn signature_answered(
        &mut self,
        id: u64,
        result: Result<Option<SignatureHelp>, IntelError>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.signatures.session.as_mut().filter(|s| s.id == id) else {
            return;
        };
        session.request = None;
        let help = result.ok().flatten().filter(|h| !h.signatures.is_empty());
        let Some(help) = help else {
            self.close_signature(cx);
            return;
        };
        session.help = Some(help);
        self.show_signature(cx);
    }

    fn show_signature(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.signatures.session else {
            return;
        };
        let Some(help) = &session.help else {
            return;
        };
        let lines = lines(help);
        let (editor, anchor) = (session.editor.clone(), session.anchor);
        self.signatures.popup.update(cx, |p, cx| p.set(lines, cx));
        let content = self.signatures.popup.clone().into();
        self.popups_mut().show(CaretPopup {
            context: CONTEXT,
            editor,
            anchor,
            placement: Placement::Above,
            inset: px(0.0),
            height: None,
            content,
        });
        cx.notify();
    }

    /// Closes parameter info; the request in flight is cancelled.
    pub(crate) fn close_signature(&mut self, cx: &mut Context<Self>) {
        let had = self.signatures.session.take().is_some();
        if self.popups_mut().hide(CONTEXT) || had {
            cx.notify();
        }
    }

    /// Closes parameter info and forgets a pending automatic one.
    pub(crate) fn reset_signature(&mut self, cx: &mut Context<Self>) {
        self.signatures.auto = None;
        self.close_signature(cx);
    }

    /// An event of a document's editor: the popup follows the caret, and
    /// typing `(` opens it after a while.
    pub(crate) fn signature_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        let ours = self
            .signatures
            .session
            .as_ref()
            .is_some_and(|s| s.editor.entity_id() == editor.entity_id());
        match event {
            EditorEvent::Edited(applied) => {
                let caret = editor.read(cx).selections().primary().head;
                let typed_paren = applied.iter().any(|e| e.text.contains('('))
                    && caret > 0
                    && editor.read(cx).buffer().char_at(caret - 1) == Some('(');
                if typed_paren {
                    self.schedule_signature(editor, caret, cx);
                }
            }
            EditorEvent::SelectionsChanged if ours => {
                let caret = editor.read(cx).selections().primary().head;
                let before_call = self
                    .signatures
                    .session
                    .as_ref()
                    .is_some_and(|s| caret <= s.anchor);
                if before_call {
                    self.close_signature(cx);
                } else {
                    self.signature_request(editor, SignatureTrigger::ContentChange, cx);
                }
            }
            EditorEvent::Reloaded => self.reset_signature(cx),
            _ => {}
        }
    }

    /// Opens parameter info after the delay, if the caret is still right
    /// after the `(` typed at `caret` then.
    fn schedule_signature(
        &mut self,
        editor: &Entity<Editor>,
        caret: usize,
        cx: &mut Context<Self>,
    ) {
        let editor = editor.clone();
        self.signatures.auto = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTO_DELAY).await;
            let _ = this.update(cx, |v, cx| {
                v.signatures.auto = None;
                let now = editor.read(cx).selections().primary().head;
                if now >= caret && call_start(editor.read(cx).buffer(), now) == Some(caret - 1) {
                    v.signature_request(&editor, SignatureTrigger::Character('('), cx);
                }
            });
        }));
    }

    /// The editor of a document lost the focus.
    pub(crate) fn signature_blurred(&mut self, editor: gpui::EntityId, cx: &mut Context<Self>) {
        if self
            .signatures
            .session
            .as_ref()
            .is_some_and(|s| s.editor.entity_id() == editor)
        {
            self.reset_signature(cx);
        }
    }

    /// The tab of a file closed.
    pub(crate) fn signature_file_closed(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if self
            .signatures
            .session
            .as_ref()
            .is_some_and(|s| s.path == path)
        {
            self.reset_signature(cx);
        }
    }

    /// The parameter info state for `code.state`.
    pub(crate) fn signature_state_json(&self, cx: &App) -> Json {
        let Some(session) = &self.signatures.session else {
            return Json::Null;
        };
        let shown = self.popups().is_shown(CONTEXT);
        let lines = self.signatures.popup.read(cx).lines.clone();
        let help = session.help.as_ref();
        let active_signature = help.map(|h| h.active_signature);
        let active_parameter = help.and_then(|h| h.active_parameter_of(h.active_signature));
        let current = lines
            .get(active_signature.unwrap_or(0))
            .and_then(|l| l.current.clone().map(|r| l.text[r].to_owned()));
        json!({
            "shown": shown,
            "anchor": session.anchor,
            "lines": lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
            "active_signature": active_signature,
            "active_parameter": active_parameter,
            "current": current,
            "pending": session.request.is_some(),
        })
    }
}

/// The content of the parameter info popup.
pub struct SignaturePopup {
    lines: Vec<Line>,
}

impl SignaturePopup {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    pub fn set(&mut self, lines: Vec<Line>, cx: &mut Context<Self>) {
        self.lines = lines;
        cx.notify();
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }
}

impl Default for SignaturePopup {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for SignaturePopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let text = theme.color(UiRole::Text);
        let muted = theme.color(UiRole::TextMuted);
        let base = font(theme.mono_family());
        let rows = self.lines.iter().map(|line| {
            let run = |len: usize, bold: bool| {
                let mut font = base.clone();
                if bold {
                    font.weight = FontWeight::BOLD;
                }
                TextRun {
                    len,
                    font,
                    color: if line.disabled { muted } else { text },
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }
            };
            let runs = match &line.current {
                Some(r) => vec![
                    run(r.start, false),
                    run(r.len(), true),
                    run(line.text.len() - r.end, false),
                ],
                None => vec![run(line.text.len(), false)],
            };
            let runs = runs.into_iter().filter(|r| r.len > 0).collect();
            div()
                .whitespace_nowrap()
                .child(StyledText::new(SharedString::from(line.text.clone())).with_runs(runs))
        });
        div()
            .debug_selector(|| "parameter-info-popup".into())
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.mono_family())
            .text_size(theme.editor_size())
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use code_intel::{ParameterInfo, Signature};

    use super::*;

    fn signature(label: &str, params: &[&str], active: Option<usize>) -> Signature {
        let mut from = 0;
        let parameters = params
            .iter()
            .map(|p| {
                let start = from + label[from..].find(p).unwrap();
                from = start + p.len();
                ParameterInfo {
                    label: start..from,
                    documentation: None,
                }
            })
            .collect();
        Signature {
            label: label.into(),
            documentation: None,
            parameters,
            active_parameter: active,
        }
    }

    #[test]
    fn names_only() {
        assert_eq!(parameter_name("obj: Any"), "obj");
        assert_eq!(parameter_name("skipkeys: bool = False"), "skipkeys");
        assert_eq!(parameter_name("*"), "*");
        assert_eq!(parameter_name("/"), "/");
        assert_eq!(parameter_name("**kwds: Any"), "kwds");
        assert_eq!(parameter_name("*args"), "args");
        assert_eq!(parameter_name("x=f(a=1)"), "x");
        assert_eq!(parameter_name("cb: Callable[[int], str] = g"), "cb");
    }

    #[test]
    fn one_line_per_signature_with_the_current_parameter() {
        let help = SignatureHelp {
            signatures: vec![
                signature(
                    "(obj: Any, *, skipkeys: bool = False, **kwds: Any) -> str",
                    &["obj: Any", "*", "skipkeys: bool = False", "**kwds: Any"],
                    None,
                ),
                signature("() -> None", &[], None),
                signature("(a, b)", &["a", "b"], Some(0)),
            ],
            active_signature: 0,
            active_parameter: Some(2),
        };
        let lines = lines(&help);
        assert_eq!(lines[0].text, "obj, *, skipkeys, kwds");
        assert_eq!(
            &lines[0].text[lines[0].current.clone().unwrap()],
            "skipkeys"
        );
        assert_eq!(lines[1].text, NO_PARAMETERS);
        assert!(lines[1].disabled && lines[1].current.is_none());
        // A signature's own current parameter wins over the help's.
        assert_eq!(&lines[2].text[lines[2].current.clone().unwrap()], "a");
    }

    #[test]
    fn the_call_around_the_caret() {
        let buffer = Buffer::new("f(a, g(b), [c, d], (e");
        // After `f(`.
        assert_eq!(call_start(&buffer, 2), Some(1));
        // After `g(b)`: the call of f again.
        assert_eq!(call_start(&buffer, 9), Some(1));
        // Inside the list: still f.
        assert_eq!(call_start(&buffer, 14), Some(1));
        // Inside `(e`.
        assert_eq!(call_start(&buffer, 21), Some(19));
        assert_eq!(call_start(&Buffer::new("x = 1"), 3), None);
    }
}
