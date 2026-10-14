//! The documentation popup, as the reference shows documentation and
//! problems: quick documentation (the module of the declaration, its
//! signature as a highlighted code block, a line, the description), the
//! popup over the text under the mouse (the problems there, each with its
//! source and rule link, an actions row with the first quick fix and More
//! actions, then the documentation of the symbol: one popup), and Error
//! Description (the problems at the caret).
//!
//! The documentation of a language service is Markdown that starts with
//! the declaration in a fenced code block (`LspDocumentationData` of the
//! reference's LSP client): that block becomes the signature section, the
//! rest the description.

use std::ops::Range;

use code_intel::{CodeAction, Markup, MarkupKind};
use editor::{DiagnosticMark, MarkSeverity};
use gpui::{
    Action, App, Context, FontWeight, InteractiveElement, InteractiveText, IntoElement,
    MouseButton, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, StyledText, TextRun, WeakEntity, Window, div, font, prelude::FluentBuilder, px,
};
use serde_json::{Value as Json, json};
use themes::{TextRole, UiRole};
use ui::markdown::{self, Code, CodeLexer, Markdown};
use ui::{Icon, IconName, ThemeExt};

use crate::commands::{ApplyFirstFix, ShowIntentions};
use crate::hook::{CodeIntelProvider, DocLocation};
use crate::view::CodeIntelView;

/// The key context of the documentation popup.
pub const CONTEXT: &str = "Documentation";

/// Widest the popup gets (the reference's documentation popup is about
/// 500 points wide); text wraps inside.
const MAX_WIDTH: f32 = 500.0;
/// Highest the popup gets; more scrolls.
const MAX_HEIGHT: f32 = 360.0;
/// Room around the sections.
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;
/// A signature of one line longer than this gets one parameter per line.
pub const SIGNATURE_WRAP: usize = 60;

/// What the popup shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// Quick Documentation at the caret.
    QuickDoc,
    /// The popup over the text under the mouse.
    Hover,
    /// Error Description: the problems at the caret.
    Problems,
}

impl DocKind {
    pub fn name(self) -> &'static str {
        match self {
            DocKind::QuickDoc => "quick-doc",
            DocKind::Hover => "hover",
            DocKind::Problems => "error-description",
        }
    }
}

/// A problem as the popup shows it: its message, then grey `Source(` +
/// the rule (a link to its documentation when there is one) + `)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub text: SharedString,
    pub source: Option<SharedString>,
    pub code: Option<SharedString>,
    pub href: Option<SharedString>,
    pub severity: MarkSeverity,
}

impl Problem {
    fn to_json(&self) -> Json {
        json!({
            "text": self.text.as_ref(),
            "source": self.source.as_ref().map(|s| s.to_string()),
            "code": self.code.as_ref().map(|s| s.to_string()),
            "href": self.href.as_ref().map(|s| s.to_string()),
        })
    }
}

/// Whether a mark is at an offset: under the mouse (`inclusive_end`
/// false: the char at the offset is in it) or at the caret (the end counts
/// too).
fn mark_at(mark: &DiagnosticMark, offset: usize, inclusive_end: bool) -> bool {
    let r = &mark.range;
    if r.is_empty() {
        return r.start == offset;
    }
    r.start <= offset && (offset < r.end || (inclusive_end && offset == r.end))
}

/// The problems of the marks at an offset, the most severe first, and the
/// range of the first of those marks.
pub fn problems_at(
    marks: &[DiagnosticMark],
    offset: usize,
    inclusive_end: bool,
) -> (Vec<Problem>, Option<Range<usize>>) {
    let mut found: Vec<&DiagnosticMark> = marks
        .iter()
        .filter(|m| mark_at(m, offset, inclusive_end))
        .collect();
    found.sort_by_key(|m| (m.severity, m.range.start));
    let range = found.first().map(|m| m.range.clone());
    let problems = found
        .iter()
        .flat_map(|m| {
            m.messages.iter().map(|message| Problem {
                text: message.text.clone(),
                source: message.source.clone(),
                code: message.code.clone(),
                href: message.href.clone(),
                severity: message.severity,
            })
        })
        .collect();
    (problems, range)
}

/// The first quick fix of the problems in the popup.
#[derive(Debug, Clone)]
pub struct Fix {
    pub action: CodeAction,
    /// The version of the document its edits are in.
    pub version: u64,
}

/// The documentation of a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Documentation {
    /// The text as the service sent it.
    pub markdown: String,
    /// The module of the declaration (the header).
    pub location: Option<DocLocation>,
    /// The declaration (the leading code block).
    pub definition: Option<Code>,
    /// The description.
    pub body: Markdown,
}

impl Documentation {
    /// The documentation of a service's text: its leading code block is
    /// the declaration, coloured by `lexer` and with one parameter per
    /// line when it is long; the rest is the description (a rule right
    /// after the declaration is left out: the popup draws its own line).
    /// `None` when there is nothing to show.
    pub fn new(
        markup: &Markup,
        location: Option<DocLocation>,
        lexer: Option<&dyn CodeLexer>,
    ) -> Option<Self> {
        if markup.is_empty() {
            return None;
        }
        let (definition, body) = match markup.kind {
            MarkupKind::PlainText => (None, Markdown::plain(&markup.text)),
            MarkupKind::Markdown => match leading_fence(&markup.text) {
                Some(fence) => {
                    let code = wrap_signature(&fence.code, SIGNATURE_WRAP);
                    let rest = strip_leading_rule(fence.rest);
                    (
                        (!code.trim().is_empty()).then(|| Code::new(fence.language, code, lexer)),
                        Markdown::parse(rest, lexer),
                    )
                }
                None => (None, Markdown::parse(&markup.text, lexer)),
            },
        };
        if definition.is_none() && body.is_empty() {
            return None;
        }
        Some(Self {
            markdown: markup.text.clone(),
            location,
            definition,
            body,
        })
    }

    fn to_json(&self) -> Json {
        json!({
            "markdown": self.markdown,
            "location": self.location.as_ref().map(|l| l.name.clone()),
            "definition": self.definition.as_ref().map(|d| d.text.clone()),
            "blocks": self.body.to_json(),
        })
    }
}

/// A fenced code block that opens a text, and the text after it.
struct Fence<'a> {
    language: Option<String>,
    code: String,
    rest: &'a str,
}

/// The fenced code block (three or more backticks) at the very start of a
/// Markdown text, closed by a fence at least as long; `None` otherwise.
fn leading_fence(text: &str) -> Option<Fence<'_>> {
    let ticks = text.chars().take_while(|&c| c == '`').count();
    if ticks < 3 {
        return None;
    }
    let info_end = text.find('\n')?;
    let language = text[ticks..info_end]
        .split_whitespace()
        .next()
        .map(str::to_owned);
    let mut at = info_end + 1;
    while at <= text.len() {
        let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let line = text[at..line_end].trim_end_matches([' ', '\t', '\r']);
        if line.len() >= ticks && line.chars().all(|c| c == '`') {
            let code = text[info_end + 1..at].trim_end().to_owned();
            let rest = text.get(line_end + 1..).unwrap_or("");
            return Some(Fence {
                language,
                code: dedent(&code),
                rest,
            });
        }
        if line_end >= text.len() {
            break;
        }
        at = line_end + 1;
    }
    None
}

/// Lines without the indentation they all share.
fn dedent(code: &str) -> String {
    let indent = code
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    code.lines()
        .map(|l| l.get(indent..).unwrap_or(l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text without a thematic break (`---`, `***`, `___`) as its first
/// line.
fn strip_leading_rule(text: &str) -> &str {
    let trimmed = text.trim_start_matches(['\n', '\r', ' ']);
    let line_end = trimmed.find('\n').unwrap_or(trimmed.len());
    let line: String = trimmed[..line_end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let is_rule = line.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| line.chars().all(|c| c.to_string() == *m));
    if is_rule {
        trimmed.get(line_end + 1..).unwrap_or("")
    } else {
        text
    }
}

/// A one-line signature longer than `limit` with one parameter per line:
/// a break after each comma between the brackets of its first parameter
/// list, the next parameter under the first one (`def dumps(obj: Any,` /
/// `          *,` / ... `          **kwds: Any) -> str`). Other code is
/// left as it is.
pub fn wrap_signature(code: &str, limit: usize) -> String {
    if code.contains('\n') || code.chars().count() <= limit {
        return code.to_owned();
    }
    let Some(open) = code.find('(') else {
        return code.to_owned();
    };
    let indent = " ".repeat(code[..=open].chars().count());
    let mut out = String::with_capacity(code.len() + 64);
    out.push_str(&code[..=open]);
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    let mut chars = code[open + 1..].char_indices().peekable();
    let mut wrapped = false;
    while let Some((i, c)) = chars.next() {
        if depth == 0 {
            out.push_str(&code[open + 1 + i..]);
            break;
        }
        out.push(c);
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth -= 1,
            (None, ',') if depth == 1 => {
                // The space after the comma goes; the next parameter
                // starts a line.
                while chars.peek().is_some_and(|(_, c)| *c == ' ') {
                    chars.next();
                }
                out.push('\n');
                out.push_str(&indent);
                wrapped = true;
            }
            _ => {}
        }
    }
    if wrapped { out } else { code.to_owned() }
}

/// What the documentation popup shows.
#[derive(Debug, Clone)]
pub struct DocContent {
    pub kind: DocKind,
    pub problems: Vec<Problem>,
    /// The first quick fix of the problems (the hover's actions row).
    pub fix: Option<Fix>,
    /// The actions row has "More actions...".
    pub more_actions: bool,
    pub documentation: Option<Documentation>,
}

impl DocContent {
    pub fn is_empty(&self) -> bool {
        self.problems.is_empty() && self.documentation.is_none()
    }

    pub fn to_json(&self) -> Json {
        json!({
            "kind": self.kind.name(),
            "problems": self.problems.iter().map(Problem::to_json).collect::<Vec<_>>(),
            "fix": self.fix.as_ref().map(|f| f.action.title.clone()),
            "more_actions": self.more_actions,
            "markdown": self.documentation.as_ref().map(|d| d.markdown.clone()),
            "documentation": self.documentation.as_ref().map(Documentation::to_json),
        })
    }
}

/// A language's lexer as the colours of code in documentation: fences
/// without a language are in `language`, the language of the document.
pub struct DocLexer<'a> {
    pub provider: &'a dyn CodeIntelProvider,
    pub language: &'a str,
}

impl CodeLexer for DocLexer<'_> {
    fn lex(&self, language: Option<&str>, code: &str) -> Vec<(Range<usize>, TextRole)> {
        let language = language.unwrap_or(self.language);
        match self.provider.lexer(language) {
            Some(lexer) => lex_lines(lexer.as_ref(), code),
            None => Vec::new(),
        }
    }
}

/// Role spans of a text from a line lexer, as byte ranges of the text.
pub fn lex_lines(lexer: &dyn syntax::Lexer, code: &str) -> Vec<(Range<usize>, TextRole)> {
    let mut spans = Vec::new();
    let mut state = lexer.initial_state();
    let mut line_spans = Vec::new();
    let mut start = 0;
    for line in code.split('\n') {
        line_spans.clear();
        state = lexer.lex_line(line, state, &mut line_spans);
        spans.extend(
            line_spans
                .iter()
                .filter(|(r, _)| r.start < r.end && r.end <= line.len())
                .map(|(r, role)| (start + r.start..start + r.end, *role)),
        );
        start += line.len() + 1;
    }
    spans
}

/// The content of the documentation popup.
pub struct DocPopup {
    owner: WeakEntity<CodeIntelView>,
    content: Option<DocContent>,
    scroll: ScrollHandle,
}

impl DocPopup {
    pub fn new(owner: WeakEntity<CodeIntelView>) -> Self {
        Self {
            owner,
            content: None,
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set(&mut self, content: DocContent, cx: &mut Context<Self>) {
        self.content = Some(content);
        self.scroll = ScrollHandle::new();
        cx.notify();
    }

    pub fn content(&self) -> Option<&DocContent> {
        self.content.as_ref()
    }

    fn render_problem(&self, index: usize, problem: &Problem, cx: &App) -> gpui::AnyElement {
        let theme = cx.theme();
        let text_color = theme.color(UiRole::Text);
        let muted = theme.color(UiRole::TextMuted);
        let link = theme.color(UiRole::TextLink);
        let base = font(theme.ui_family());
        let run = |len: usize, color| TextRun {
            len,
            font: base.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut text = problem.text.to_string();
        let mut runs = vec![run(text.len(), text_color)];
        let mut code_range = None;
        if let Some(source) = &problem.source {
            // A non-breaking space keeps the source with the message.
            let prefix = match &problem.code {
                Some(_) => format!("\u{a0}{source}("),
                None => format!("\u{a0}{source}"),
            };
            runs.push(run(prefix.len(), muted));
            text.push_str(&prefix);
        }
        if let Some(code) = &problem.code {
            let start = text.len();
            if problem.source.is_none() {
                text.push('\u{a0}');
                runs.push(run(text.len() - start, muted));
            }
            let code_start = text.len();
            text.push_str(code);
            let color = if problem.href.is_some() { link } else { muted };
            runs.push(run(code.len(), color));
            code_range = Some(code_start..text.len());
            if problem.source.is_some() {
                text.push(')');
                runs.push(run(1, muted));
            }
        }
        runs.retain(|r| r.len > 0);
        let styled = StyledText::new(SharedString::from(text)).with_runs(runs);
        let element = match (code_range, problem.href.clone()) {
            (Some(range), Some(href)) => InteractiveText::new(("doc-problem", index), styled)
                .on_click(vec![range], move |_, _, cx| markdown::open_link(&href, cx))
                .into_any_element(),
            _ => styled.into_any_element(),
        };
        div()
            .debug_selector(move || format!("doc-problem-{index}"))
            .px(px(PAD_X))
            .py(px(PAD_Y))
            .child(element)
            .into_any_element()
    }

    fn render_actions(&self, content: &DocContent, window: &Window, cx: &App) -> gpui::AnyElement {
        let theme = cx.theme();
        let link = theme.color(UiRole::TextLink);
        let muted = theme.color(UiRole::TextMuted);
        let shortcut = |action: &dyn Action| {
            window
                .highest_precedence_binding_for_action(action)
                .map(|b| ui::keys::binding_text(&b))
        };
        let owner = self.owner.clone();
        let fix = content.fix.as_ref().map(|fix| {
            let owner = owner.clone();
            div()
                .flex()
                .flex_row()
                .gap(px(6.0))
                .child(
                    div()
                        .id("doc-fix")
                        .debug_selector(|| "doc-fix".into())
                        .text_color(link)
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |v, cx| v.apply_doc_fix(window, cx));
                            }
                        })
                        .child(fix.action.title.clone()),
                )
                .children(shortcut(&ApplyFirstFix).map(|s| div().text_color(muted).child(s)))
        });
        let more = content.more_actions.then(|| {
            let owner = owner.clone();
            div()
                .flex()
                .flex_row()
                .gap(px(6.0))
                .child(
                    div()
                        .id("doc-more-actions")
                        .debug_selector(|| "doc-more-actions".into())
                        .text_color(link)
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |v, cx| v.close_doc(cx));
                            }
                            window.dispatch_action(ShowIntentions.boxed_clone(), cx);
                        })
                        .child("More actions..."),
                )
                .children(shortcut(&ShowIntentions).map(|s| div().text_color(muted).child(s)))
        });
        div()
            .debug_selector(|| "doc-actions".into())
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_x(px(16.0))
            .gap_y(px(4.0))
            .px(px(PAD_X))
            .py(px(PAD_Y))
            .children(fix)
            .children(more)
            .into_any_element()
    }

    fn render_documentation(&self, doc: &Documentation, cx: &App) -> gpui::AnyElement {
        let theme = cx.theme();
        let header = doc.location.as_ref().map(|location| {
            div()
                .debug_selector(|| "doc-location".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(Icon::new(location.icon).size(px(16.0)))
                .child(location.name.clone())
        });
        let definition = doc.definition.as_ref().map(|code| {
            div()
                .debug_selector(|| "doc-definition".into())
                .child(markdown::render_code(code, cx))
        });
        let separator = (doc.definition.is_some() && !doc.body.is_empty())
            .then(|| div().w_full().h(px(1.0)).bg(theme.color(UiRole::Divider)));
        let body = (!doc.body.is_empty()).then(|| {
            div()
                .debug_selector(|| "doc-body".into())
                .child(doc.body.render("doc-link", cx))
        });
        div()
            .debug_selector(|| "doc-documentation".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(PAD_X))
            .py(px(PAD_Y))
            .children(header)
            .children(definition)
            .children(separator)
            .children(body)
            .into_any_element()
    }
}

impl Render for DocPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let Some(content) = &self.content else {
            return div().into_any_element();
        };
        let divider = || {
            div()
                .w_full()
                .h(px(1.0))
                .bg(theme.color(UiRole::PopupBorder))
        };
        let mut sections: Vec<gpui::AnyElement> = Vec::new();
        for (i, problem) in content.problems.iter().enumerate() {
            if i > 0 {
                sections.push(divider().into_any_element());
            }
            sections.push(self.render_problem(i, problem, cx));
        }
        if content.fix.is_some() || content.more_actions {
            sections.push(self.render_actions(content, window, cx));
        }
        if let Some(doc) = &content.documentation {
            if !sections.is_empty() {
                sections.push(divider().into_any_element());
            }
            sections.push(self.render_documentation(doc, cx));
        }
        let owner = self.owner.clone();
        div()
            .id("documentation-popup")
            .debug_selector(|| "documentation-popup".into())
            .max_w(px(MAX_WIDTH))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::DocumentationPopup))
            .shadow(ui::components::popup_shadow())
            .overflow_hidden()
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .font_weight(FontWeight::NORMAL)
            .on_hover(move |inside, _, cx| {
                if let Some(owner) = owner.upgrade() {
                    owner.update(cx, |v, cx| v.doc_mouse_inside(*inside, cx));
                }
            })
            .child(
                div()
                    .id("documentation-scroll")
                    .max_h(px(MAX_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .when(sections.is_empty(), |d| d.p(px(PAD_Y)))
                    .children(sections),
            )
            .into_any_element()
    }
}

/// The header of documentation by default: the declaring file's name
/// without its extension, with the module icon.
pub fn default_location(path: &std::path::Path) -> Option<DocLocation> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(DocLocation {
        name,
        icon: IconName::MODULE,
    })
}

#[cfg(test)]
mod tests {
    use editor::MarkMessage;

    use super::*;

    #[test]
    fn the_leading_fence_is_the_declaration() {
        let text = "```python\ndef dumps(obj: Any, *, skipkeys: bool = False, ensure_ascii: bool = True, **kwds: Any) -> str\n```\n---\nSerialize ``obj`` to a JSON formatted ``str``.";
        let doc = Documentation::new(&Markup::markdown(text), None, None).unwrap();
        let definition = doc.definition.as_ref().unwrap();
        assert_eq!(definition.language.as_deref(), Some("python"));
        assert_eq!(
            definition.text,
            "def dumps(obj: Any,\n          *,\n          skipkeys: bool = False,\n          ensure_ascii: bool = True,\n          **kwds: Any) -> str"
        );
        // The rule after the declaration is the popup's own line.
        let json = doc.body.to_json();
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["text"], "Serialize obj to a JSON formatted str.");

        // Without a fence everything is the description.
        let doc = Documentation::new(&Markup::markdown("Just *text*."), None, None).unwrap();
        assert!(doc.definition.is_none());
        let doc = Documentation::new(&Markup::plain("a\n\nb"), None, None).unwrap();
        assert_eq!(doc.body.blocks.len(), 2);
        assert!(Documentation::new(&Markup::markdown("  "), None, None).is_none());
        // A fence that is not closed is not a declaration.
        assert!(leading_fence("```python\ndef f(): ...").is_none());
        let fence = leading_fence("````\n  x = 1\n  y\n````  \nrest").unwrap();
        assert_eq!(fence.language, None);
        assert_eq!(fence.code, "x = 1\ny");
        assert_eq!(fence.rest, "rest");
    }

    #[test]
    fn long_signatures_get_a_parameter_per_line() {
        assert_eq!(wrap_signature("def f(a, b)", 60), "def f(a, b)");
        assert_eq!(
            wrap_signature("def f(a: dict[str, int] = {'x,': 1}, b=(1, 2)) -> None", 10),
            "def f(a: dict[str, int] = {'x,': 1},\n      b=(1, 2)) -> None"
        );
        assert_eq!(wrap_signature("class X: ...", 3), "class X: ...");
        assert_eq!(wrap_signature("x\ny", 0), "x\ny");
    }

    fn mark(range: Range<usize>, severity: MarkSeverity, text: &str) -> DiagnosticMark {
        DiagnosticMark {
            range,
            severity,
            unnecessary: false,
            deprecated: false,
            messages: vec![MarkMessage {
                severity,
                text: text.to_owned().into(),
                source: Some("Ruff".into()),
                code: Some("unused-import".into()),
                href: None,
            }],
        }
    }

    #[test]
    fn problems_under_the_mouse_and_at_the_caret() {
        let marks = [
            mark(0..9, MarkSeverity::Warning, "unused"),
            mark(7..9, MarkSeverity::Error, "unresolved"),
            mark(12..12, MarkSeverity::Information, "hint"),
        ];
        let (problems, range) = problems_at(&marks, 7, false);
        let texts: Vec<&str> = problems.iter().map(|p| p.text.as_ref()).collect();
        assert_eq!(texts, ["unresolved", "unused"]);
        assert_eq!(range, Some(7..9));
        assert!(problems_at(&marks, 9, false).0.is_empty());
        assert_eq!(problems_at(&marks, 9, true).0.len(), 2);
        assert_eq!(problems_at(&marks, 12, false).0.len(), 1);
    }

    struct Keywords;

    impl syntax::Lexer for Keywords {
        fn lex_line(&self, line: &str, state: u32, out: &mut Vec<syntax::LexSpan>) -> u32 {
            if let Some(i) = line.find("def") {
                out.push((i..i + 3, TextRole::Keyword));
            }
            state + 1
        }
    }

    #[test]
    fn line_lexers_colour_whole_blocks() {
        let spans = lex_lines(&Keywords, "def f(a,\n      b): def");
        assert_eq!(
            spans,
            [(0..3, TextRole::Keyword), (19..22, TextRole::Keyword)]
        );
    }
}
