//! The console of a Run tab: the `console` crate's text model shown in a
//! read-only editor (ADR 0007), as the reference IDE's `ConsoleViewImpl`
//! shows its document.
//!
//! The model is fed off the UI thread (link filters may look at the file
//! system); the view copies what changed into its editor at most once per
//! frame, with the colours of the lines' content kinds and ANSI styles and
//! the links. Typed text goes to the process's standard input: it is echoed
//! at the end of the output in the user input colour and sent line by line
//! on Enter (the console has no terminal; the process does not echo).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use console::{Color, ContentKind, LinkTarget, Style, Underline};
use editor::{Editor, EditorEvent, ForwardedInput, Highlighter, InlineStyle, Span};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use process::ProcessHandle;
use text::{Buffer, Edit, Selection, SelectionSet};
use themes::{TextRole, UiRole};
use ui::theme::{Theme, color};

/// How long output waits before it is shown: about one frame, so that a
/// process printing at full speed costs one repaint per frame.
pub const FLUSH_DELAY: Duration = Duration::from_millis(16);

/// One style run of a console line, in chars.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    range: Range<usize>,
    kind: ContentKind,
    style: Style,
}

/// What the view keeps of a console line for drawing it.
#[derive(Debug, Clone, Default, PartialEq)]
struct LineDecor {
    runs: Vec<Run>,
    links: Vec<(Range<usize>, LinkTarget)>,
}

impl LineDecor {
    fn of(line: &console::Line) -> Self {
        let text = line.text();
        // Byte offsets to char offsets, in one pass per line.
        let to_char = |byte: usize| text[..byte.min(text.len())].chars().count();
        Self {
            runs: line
                .runs()
                .iter()
                .map(|r| Run {
                    range: to_char(r.range.start)..to_char(r.range.end),
                    kind: r.kind,
                    style: r.style,
                })
                .collect(),
            links: line
                .links()
                .iter()
                .map(|l| {
                    (
                        to_char(l.range.start)..to_char(l.range.end),
                        l.target.clone(),
                    )
                })
                .collect(),
        }
    }
}

type Decor = Rc<RefCell<Vec<LineDecor>>>;

/// The text role of plain text of a content kind (normal output has the
/// console's default colour).
fn kind_role(kind: ContentKind) -> Option<TextRole> {
    match kind {
        ContentKind::Normal => None,
        ContentKind::Error => Some(TextRole::ConsoleError),
        ContentKind::System => Some(TextRole::ConsoleSystem),
        ContentKind::UserInput => Some(TextRole::ConsoleInput),
    }
}

/// The colour of an ANSI colour in the theme.
pub fn ansi_color(c: Color, theme: &Theme) -> Hsla {
    const ROLES: [UiRole; 16] = [
        UiRole::TerminalBlack,
        UiRole::TerminalRed,
        UiRole::TerminalGreen,
        UiRole::TerminalYellow,
        UiRole::TerminalBlue,
        UiRole::TerminalMagenta,
        UiRole::TerminalCyan,
        UiRole::TerminalWhite,
        UiRole::TerminalBrightBlack,
        UiRole::TerminalBrightRed,
        UiRole::TerminalBrightGreen,
        UiRole::TerminalBrightYellow,
        UiRole::TerminalBrightBlue,
        UiRole::TerminalBrightMagenta,
        UiRole::TerminalBrightCyan,
        UiRole::TerminalBrightWhite,
    ];
    let rgb = |r: u8, g: u8, b: u8| {
        gpui::Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
        .into()
    };
    match c {
        Color::Ansi(i) => theme.color(ROLES[(i as usize).min(15)]),
        Color::Indexed(i) => Color::indexed_rgb(i).map_or_else(
            || theme.color(ROLES[(i as usize).min(15)]),
            |(r, g, b)| rgb(r, g, b),
        ),
        Color::Rgb(r, g, b) => rgb(r, g, b),
    }
}

/// The reference's drawing of ANSI text (`ColoredOutputTypeRegistryImpl`):
/// the style's colours, or the normal output colour and the console
/// background, swapped when inverse.
fn ansi_style(style: &Style, theme: &Theme) -> InlineStyle {
    let fg = style.fg.map(|c| ansi_color(c, theme));
    let bg = style.bg.map(|c| ansi_color(c, theme));
    let (fg, bg) = if style.inverse {
        (
            Some(bg.unwrap_or_else(|| theme.color(UiRole::TerminalBackground))),
            Some(fg.unwrap_or_else(|| theme.color(UiRole::TerminalForeground))),
        )
    } else {
        (
            Some(fg.unwrap_or_else(|| theme.color(UiRole::TerminalForeground))),
            bg,
        )
    };
    InlineStyle {
        color: fg,
        background: bg,
        bold: style.bold,
        italic: style.italic,
        underline: match style.underline {
            Underline::None => None,
            Underline::Single => Some(1.0),
            Underline::Double => Some(2.0),
        },
        strikethrough: style.strikethrough,
    }
}

/// Styles and links of the console's lines for its editor.
struct ConsoleHighlighter {
    decor: Decor,
}

impl Highlighter for ConsoleHighlighter {
    fn reset(&mut self, _: &Buffer) {}

    fn edited(&mut self, _: &Buffer, _: &[text::AppliedEdit]) {}

    fn line_spans(&self, line: usize, out: &mut Vec<Span>) {
        if let Some(d) = self.decor.borrow().get(line) {
            for run in &d.runs {
                if !run.style.ansi
                    && let Some(role) = kind_role(run.kind)
                {
                    out.push((run.range.clone(), role));
                }
            }
        }
    }

    fn line_styles(&self, line: usize, theme: &Theme, out: &mut Vec<(Range<usize>, InlineStyle)>) {
        if let Some(d) = self.decor.borrow().get(line) {
            for run in &d.runs {
                if run.style.ansi {
                    out.push((run.range.clone(), ansi_style(&run.style, theme)));
                }
            }
        }
    }

    fn line_links(&self, line: usize, out: &mut Vec<Range<usize>>) {
        if let Some(d) = self.decor.borrow().get(line) {
            out.extend(d.links.iter().map(|(r, _)| r.clone()));
        }
    }
}

/// What the console asks of its tab.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleEvent {
    /// A link was followed.
    OpenLink(LinkTarget),
}

/// A Run tab's console.
pub struct ConsoleView {
    editor: Entity<Editor>,
    model: Arc<Mutex<console::Console>>,
    /// New output waits for the view (set by the feeding thread).
    dirty: Arc<AtomicBool>,
    decor: Decor,
    /// The process that gets typed input while it runs.
    input: Option<ProcessHandle>,
    /// Typed text of the current line not sent yet.
    pending_input: String,
    flush_scheduled: bool,
    /// The link followed last (line, char range), where Up and Down the
    /// Stack Trace continue.
    followed: Option<(usize, Range<usize>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConsoleEvent> for ConsoleView {}

impl ConsoleView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let decor: Decor = Rc::new(RefCell::new(vec![LineDecor::default()]));
        let editor = cx.new(|cx| {
            let mut buffer = Buffer::new("");
            buffer.set_read_only(true);
            let mut editor = Editor::new(buffer, window, cx);
            editor.set_console(true);
            editor.set_forward_input(true);
            editor.set_highlighter(
                Some(Box::new(ConsoleHighlighter {
                    decor: decor.clone(),
                })),
                cx,
            );
            editor
        });
        let subscriptions =
            vec![
                cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| match event {
                    EditorEvent::Input(input) => this.input(input.clone(), cx),
                    EditorEvent::LinkClicked { line, range } => {
                        this.follow(*line, range.clone(), cx)
                    }
                    _ => {}
                }),
            ];
        Self {
            editor,
            model: Arc::new(Mutex::new(console::Console::new())),
            dirty: Arc::new(AtomicBool::new(false)),
            decor,
            input: None,
            pending_input: String::new(),
            flush_scheduled: false,
            followed: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The text model, shared with the thread that feeds it.
    pub fn model(&self) -> Arc<Mutex<console::Console>> {
        self.model.clone()
    }

    /// Set by the feeding thread when it changed the model; the view
    /// clears it when it takes the changes.
    pub fn dirty_flag(&self) -> Arc<AtomicBool> {
        self.dirty.clone()
    }

    fn lock(&self) -> MutexGuard<'_, console::Console> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The process that gets typed input (`None` once it ended).
    pub fn set_input(&mut self, input: Option<ProcessHandle>) {
        self.input = input;
        self.pending_input.clear();
    }

    /// Starts over for a new run: no text, these link filters.
    pub fn reset(&mut self, filters: Vec<Arc<dyn console::LinkFilter>>, cx: &mut Context<Self>) {
        {
            let mut model = self.lock();
            model.clear();
            model.set_link_filters(filters);
        }
        self.followed = None;
        self.pending_input.clear();
        self.flush(cx);
    }

    /// Prints the IDE's own text (system output), shown at once.
    pub fn print_system(&mut self, text: &str, cx: &mut Context<Self>) {
        self.lock().push_str(ContentKind::System, text);
        self.flush(cx);
    }

    /// New output is waiting: show it within a frame.
    pub fn request_flush(&mut self, cx: &mut Context<Self>) {
        if self.flush_scheduled {
            return;
        }
        self.flush_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLUSH_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.flush_scheduled = false;
                this.flush(cx);
            });
        })
        .detach();
    }

    /// Copies what changed in the model into the editor. The view sticks
    /// to the end when it showed the end before (`isStickingToEnd`).
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.dirty.store(false, Ordering::Release);
        // The feeding thread holds the model only for a chunk; try again
        // next frame instead of waiting for it.
        let model = self.model.clone();
        let mut model = match model.try_lock() {
            Ok(m) => m,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.dirty.store(true, Ordering::Release);
                self.request_flush(cx);
                return;
            }
        };
        let Some(changes) = model.take_changes() else {
            return;
        };
        let editor = self.editor.clone();
        let stick = editor.read(cx).is_scrolled_to_end();
        let joined = |model: &console::Console, from: usize| -> String {
            let mut out = String::new();
            for (i, line) in model.lines().skip(from).enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line.text());
            }
            out
        };
        let mut decor = self.decor.borrow_mut();
        editor.update(cx, |e, cx| {
            let lines = e.buffer().len_lines();
            if changes.cleared || changes.trimmed >= lines {
                let len = e.buffer().len_chars();
                e.replace_untracked(vec![Edit::new(0..len, joined(&model, 0))], cx);
                *decor = model.lines().map(LineDecor::of).collect();
                return;
            }
            if changes.trimmed > 0 {
                let end = e.buffer().line_start(changes.trimmed);
                e.replace_untracked(vec![Edit::delete(0..end)], cx);
                let trimmed = changes.trimmed.min(decor.len());
                decor.drain(..trimmed);
            }
            if changes.first_line_cut && changes.changed_from > 0 {
                let end = e.buffer().line_end(0);
                let line = model
                    .line(0)
                    .map(|l| l.text().to_owned())
                    .unwrap_or_default();
                e.replace_untracked(vec![Edit::new(0..end, line)], cx);
                if let (Some(d), Some(l)) = (decor.first_mut(), model.line(0)) {
                    *d = LineDecor::of(l);
                }
            }
            let from = changes.changed_from.min(model.line_count());
            let lines = e.buffer().len_lines();
            let len = e.buffer().len_chars();
            let edit = if from < lines {
                Edit::new(e.buffer().line_start(from)..len, joined(&model, from))
            } else {
                Edit::insert(len, format!("\n{}", joined(&model, from)))
            };
            e.replace_untracked(vec![edit], cx);
            decor.truncate(from);
            decor.extend(model.lines().skip(from).map(LineDecor::of));
        });
        drop(decor);
        drop(model);
        if changes.trimmed > 0 {
            self.followed = self.followed.take().and_then(|(line, range)| {
                line.checked_sub(changes.trimmed).map(|line| (line, range))
            });
        }
        if stick {
            editor.update(cx, |e, cx| {
                if e.selections().primary().is_empty() {
                    let end = e.buffer().len_chars();
                    e.set_selections(SelectionSet::caret(end), cx);
                }
                e.scroll_to_end(cx);
            });
        }
        cx.notify();
    }

    /// Typed input: echoed in the user input colour; Enter sends the line
    /// (`ConsoleViewImpl.type`, `EnterHandler`, `sendUserInput`). Typing
    /// works only while the process runs.
    fn input(&mut self, input: ForwardedInput, cx: &mut Context<Self>) {
        let Some(process) = self.input.clone().filter(|p| p.is_running()) else {
            return;
        };
        match input {
            ForwardedInput::Text(text) => {
                for (i, part) in text.split('\n').enumerate() {
                    if i > 0 {
                        self.send_line(&process, cx);
                    }
                    if !part.is_empty() {
                        self.pending_input.push_str(part);
                        self.lock().push_str(ContentKind::UserInput, part);
                    }
                }
            }
            ForwardedInput::Enter => self.send_line(&process, cx),
            ForwardedInput::Tab => {
                self.pending_input.push('\t');
                self.lock().push_str(ContentKind::UserInput, "\t");
            }
            ForwardedInput::Backspace => {
                if self.pending_input.pop().is_some() {
                    self.lock().push_str(ContentKind::UserInput, "\u{8}");
                }
            }
            ForwardedInput::Delete => {}
        }
        self.flush(cx);
        // The caret goes to the end of the input (`moveScrollRemoveSelection`).
        self.editor.update(cx, |e, cx| {
            let end = e.buffer().len_chars();
            e.set_selections(SelectionSet::caret(end), cx);
            e.scroll_to_end(cx);
        });
    }

    fn send_line(&mut self, process: &ProcessHandle, _: &mut Context<Self>) {
        self.lock().push_str(ContentKind::UserInput, "\n");
        let mut line = std::mem::take(&mut self.pending_input);
        line.push('\n');
        let _ = process.write_stdin(line.as_bytes());
    }

    /// Types text as the person would (the `run.console.type` command).
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input(ForwardedInput::Text(text.to_owned()), cx);
    }

    /// Closes the process's standard input (Send EOF).
    pub fn send_eof(&mut self) {
        if let Some(p) = &self.input {
            p.close_stdin();
        }
    }

    fn follow(&mut self, line: usize, range: Range<usize>, cx: &mut Context<Self>) {
        let target = self.decor.borrow().get(line).and_then(|d| {
            d.links
                .iter()
                .find(|(r, _)| *r == range)
                .map(|(_, t)| t.clone())
        });
        if let Some(target) = target {
            self.followed = Some((line, range));
            cx.emit(ConsoleEvent::OpenLink(target));
        }
    }

    /// Goes to the next (or previous) link after the one followed last,
    /// round the end, and follows it (`calcNextOccurrence`).
    pub fn go_to_link(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let links: Vec<(usize, Range<usize>)> = self
            .decor
            .borrow()
            .iter()
            .enumerate()
            .flat_map(|(i, d)| d.links.iter().map(move |(r, _)| (i, r.clone())))
            .collect();
        if links.is_empty() {
            return false;
        }
        let current = self
            .followed
            .as_ref()
            .and_then(|f| links.iter().position(|l| l == f));
        let next = match (current, forward) {
            (Some(i), true) => (i + 1) % links.len(),
            (Some(i), false) => (i + links.len() - 1) % links.len(),
            (None, true) => 0,
            (None, false) => links.len() - 1,
        };
        let (line, range) = links[next].clone();
        self.editor.update(cx, |e, cx| {
            let start = e.buffer().line_start(line) + range.start;
            e.set_selections(SelectionSet::single(Selection::caret(start)), cx);
            e.reveal(start, cx);
        });
        self.follow(line, range, cx);
        true
    }

    /// Scrolls to the end and puts the caret there (`EditorUtil.scrollToTheEnd`).
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            let end = e.buffer().len_chars();
            e.set_selections(SelectionSet::caret(end), cx);
            e.scroll_to_end(cx);
        });
    }

    /// Removes all text (Clear All).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.lock().clear();
        self.pending_input.clear();
        self.followed = None;
        self.flush(cx);
    }

    /// The console has links (Up and Down the Stack Trace are enabled).
    pub fn has_links(&self) -> bool {
        self.decor.borrow().iter().any(|d| !d.links.is_empty())
    }

    /// The console has text (Clear All is enabled).
    pub fn has_content(&self, cx: &App) -> bool {
        self.editor.read(cx).buffer().len_chars() > 0
    }

    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).text()
    }

    /// The state reported by `run.state`: the lines with their kinds and
    /// links.
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let text = self.text(cx);
        let decor = self.decor.borrow();
        let kind = |k: ContentKind| match k {
            ContentKind::Normal => "normal",
            ContentKind::Error => "error",
            ContentKind::System => "system",
            ContentKind::UserInput => "input",
        };
        let lines: Vec<serde_json::Value> = text
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                let d = decor.get(i);
                serde_json::json!({
                    "text": line,
                    "runs": d.map(|d| d.runs.iter().map(|r| serde_json::json!({
                        "start": r.range.start,
                        "end": r.range.end,
                        "kind": kind(r.kind),
                        "ansi": r.style.ansi,
                    })).collect::<Vec<_>>()),
                    "links": d.map(|d| d.links.iter().map(|(r, t)| serde_json::json!({
                        "start": r.start,
                        "end": r.end,
                        "target": match t {
                            LinkTarget::File { path, line, column } => serde_json::json!({
                                "file": path.display().to_string(),
                                "line": line,
                                "column": column,
                            }),
                            LinkTarget::Url(url) => serde_json::json!({ "url": url }),
                        },
                    })).collect::<Vec<_>>()),
                })
            })
            .collect();
        serde_json::json!({
            "lines": lines,
            "pending_input": self.pending_input,
            "scrolled_to_end": self.editor.read(cx).is_scrolled_to_end(),
        })
    }
}

impl Focusable for ConsoleView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for ConsoleView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("RunConsole")
            .size_full()
            .child(self.editor.clone())
    }
}

/// The text colour of a theme text role, for tests and tools.
pub fn role_color(role: TextRole, theme: &Theme) -> Option<Hsla> {
    theme.text_style(role).fg.map(color)
}
