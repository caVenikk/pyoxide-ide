//! A console: its file, its session, the output log and the result tabs.
//! It is a GPUI entity of its own so that the Services tool window and the
//! toolbar of its editor observe it; events of its session come in through
//! [`super::route`].

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use console::ContentKind;
use console_ui::ConsoleView;
use db_core::Backend;
use db_session::{
    DataSourceHandle, ErrorPolicy, Event, EventKind, Isolation, Job, JobEvent, JobId, JobOutcome,
    JobStatement, ResultId, Session, SessionEvent, SessionOptions, StatementError, Timing,
    TxStatus,
};
use editor::{Decoration, Editor, EditorEvent, TextPiece};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, SharedString,
    Subscription, WeakEntity, Window,
};
use serde_json::{Value as Json, json};
use sql_syntax::{Dialect, TokenKind};
use themes::{TextRole, UiRole};
use workbench::Workbench;

use crate::grid::{GridTarget, TableGrid};
use crate::sql::Hub;
use crate::sql::hub::{ConsoleBinding, Source};

pub(crate) mod safety;
mod tx;

pub(crate) use tx::parse_mode;

/// What opens a console: the data source of the Database tool window.
#[derive(Clone)]
pub(crate) struct ConsoleTarget {
    pub source: String,
    pub backend: Backend,
    /// The database, service or file the data source points at (the
    /// prompt of the output log and the chip show it).
    pub database: Option<String>,
    pub handle: DataSourceHandle,
}

/// What the console is of: a console file of the data source (bound to it
/// by where it lives), or a `.sql` file of the project attached to a data
/// source, which has its own session and its own node in the Services
/// tree but stays an ordinary file for the SQL glue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsoleKind {
    Console,
    File,
}

/// What the Services tool window shows of a console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Output,
    Result(usize),
}

/// A result tab under the console's editor.
pub(crate) struct ResultTab {
    pub title: SharedString,
    pub pinned: bool,
    /// The user gave the tab its name: it is kept, never reused.
    pub renamed: bool,
    pub grid: Entity<TableGrid>,
    /// The session's result the grid shows (to route its pages).
    pub result: Option<ResultId>,
}

/// The statement being run, for the lines it leaves in the output.
struct Current {
    text: String,
    range: Range<usize>,
    /// Rows of the first page, when the statement returned a result.
    rows: Option<usize>,
    affected: Option<u64>,
}

/// The group of the editor decorations a console draws.
pub(crate) const MARK_GROUP: &str = "db-console-run";

/// The statement the editor shows the marks of: the box around it, the
/// check mark in the gutter and the time after it.
struct Mark {
    /// Char range of the statement in the editor's text, kept through
    /// edits.
    range: Range<usize>,
    editor: WeakEntity<Editor>,
    _subscription: Subscription,
}

/// An event of the console for its observers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConsoleEvent {
    /// A statement started: the Services tool window shows.
    Started,
}

pub(crate) struct Console {
    pub path: PathBuf,
    pub kind: ConsoleKind,
    pub source: String,
    pub backend: Backend,
    database: Option<String>,
    handle: DataSourceHandle,
    pub session: Session,
    pub window: AnyWindowHandle,
    pub workbench: WeakEntity<Workbench>,
    hub: Arc<Hub>,
    pub output: Entity<ConsoleView>,
    pub tabs: Vec<ResultTab>,
    pub pane: Pane,
    pub schema: Option<String>,
    pub tx: TxStatus,
    pub isolation: Isolation,
    pending_isolation: Option<Isolation>,
    /// Values typed into the Parameters dialog, by parameter: the next run
    /// starts from them.
    pub param_values: HashMap<String, String>,
    /// The statements the unsafe-query banner waits for.
    pub unsafe_run: Option<safety::UnsafeRun>,
    pub running: Option<JobId>,
    /// How long the last statement took, for the Services tree.
    pub last_ms: Option<u64>,
    current: Option<Current>,
    mark: Option<Mark>,
    /// The output log wraps long lines.
    pub soft_wrap: bool,
    /// Wall time of every line is taken when it is written.
    lines: Vec<String>,
}

impl EventEmitter<ConsoleEvent> for Console {}

impl Console {
    pub fn new(
        path: PathBuf,
        target: ConsoleTarget,
        kind: ConsoleKind,
        workbench: WeakEntity<Workbench>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let hub = cx.global::<crate::sql::widget::SqlGlobal>().0.clone();
        let output = cx.new(|cx| {
            let mut view = ConsoleView::new(window, cx);
            view.set_key_context("DatabaseOutput");
            view
        });
        let session = target.handle.open_session(SessionOptions::default());
        let console = Self {
            path,
            kind,
            source: target.source,
            backend: target.backend,
            database: target.database,
            handle: target.handle,
            session,
            window: window.window_handle(),
            workbench,
            hub,
            output,
            tabs: Vec::new(),
            pane: Pane::Output,
            schema: None,
            tx: TxStatus::default(),
            isolation: Isolation::default(),
            pending_isolation: None,
            param_values: HashMap::new(),
            unsafe_run: None,
            running: None,
            last_ms: None,
            current: None,
            mark: None,
            soft_wrap: false,
            lines: Vec::new(),
        };
        console.bind();
        console
    }

    pub fn dialect(&self) -> Dialect {
        crate::sql::hub::syntax_dialect(self.backend.dialect())
    }

    /// The name of the console's node in the Services tree: `console`, or
    /// the name of the attached file.
    pub fn label(&self) -> String {
        match self.kind {
            ConsoleKind::Console => "console".to_owned(),
            ConsoleKind::File => self.path.file_name().map_or_else(
                || "console".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            ),
        }
    }

    /// Tells the SQL glue which data source (and schema) the file is of; a
    /// file attached to a data source is bound by its mapping already.
    fn bind(&self) {
        if self.kind == ConsoleKind::File {
            return;
        }
        self.hub.bind_console(
            &self.path,
            ConsoleBinding {
                source: Source {
                    name: self.source.clone(),
                    dialect: self.dialect(),
                    database: self.database.clone(),
                },
                schema: self.schema.clone(),
            },
        );
    }

    /// `database.schema> ` before an echoed statement.
    fn prompt(&self) -> String {
        let database = self.database.as_deref().unwrap_or(&self.source);
        match &self.schema {
            Some(schema) => format!("{database}.{schema}> "),
            None => format!("{database}> "),
        }
    }

    // ---- output -----------------------------------------------------------------

    pub(super) fn print(&mut self, kind: ContentKind, text: &str, cx: &mut Context<Self>) {
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{stamp}] {text}\n");
        self.lines.push(line.clone());
        self.output.update(cx, |view, cx| {
            view.model()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(kind, &line);
            view.flush(cx);
        });
        cx.notify();
    }

    // ---- the output log's toolbar -------------------------------------------------

    pub fn clear_output(&mut self, cx: &mut Context<Self>) {
        self.lines.clear();
        self.output.update(cx, |v, cx| v.clear(cx));
        cx.notify();
    }

    pub fn scroll_output_to_end(&mut self, cx: &mut Context<Self>) {
        self.output.update(cx, |v, cx| v.scroll_to_end(cx));
    }

    pub fn toggle_soft_wrap(&mut self, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        let on = self.soft_wrap;
        let editor = self.output.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.set_soft_wraps(on, cx));
        cx.notify();
    }

    // ---- running ----------------------------------------------------------------

    /// Runs statements, their values bound, as one job in a script's order.
    pub(super) fn run_jobs(&mut self, statements: Vec<JobStatement>, cx: &mut Context<Self>) {
        if statements.is_empty() {
            return;
        }
        if self.running.is_some() {
            self.print(ContentKind::Error, "A statement is still running", cx);
            return;
        }
        let job = Job::statements(statements).on_error(ErrorPolicy::Stop);
        self.running = Some(self.session.execute(job));
        self.pane = Pane::Output;
        cx.emit(ConsoleEvent::Started);
        cx.notify();
    }

    /// Runs the statement at the caret, or the selection's statements.
    pub fn execute_at_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self
            .workbench
            .upgrade()
            .and_then(|wb| wb.read(cx).editor_for(&self.path))
        else {
            return;
        };
        let (text, selection) = {
            let editor = editor.read(cx);
            let text = editor.text();
            let sel = editor.selections().primary();
            let (range, caret) = (sel.range(), sel.head);
            let chars = |c: usize| char_to_byte(&text, c);
            (
                text.clone(),
                (chars(range.start)..chars(range.end), chars(caret)),
            )
        };
        let dialect = self.dialect();
        let (range, caret) = selection;
        let statements = if range.is_empty() {
            sql_syntax::statement_at(&text, caret, dialect)
                .into_iter()
                .collect()
        } else {
            sql_syntax::statements_in(&text, range, dialect)
        };
        self.submit(statements, window, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            self.session.cancel();
            cx.notify();
        }
    }

    // ---- events -----------------------------------------------------------------

    pub fn on_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event.kind {
            EventKind::Session { event, .. } => self.on_session(event, cx),
            EventKind::Job { job, event, .. } => self.on_job(job, event, window, cx),
            _ => {}
        }
    }

    fn on_session(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Connected => self.print(ContentKind::System, "Connected", cx),
            SessionEvent::Schema { schema } => {
                self.schema = Some(schema);
                self.bind();
                cx.notify();
            }
            SessionEvent::Transaction { status } => {
                let old = std::mem::replace(&mut self.tx, status);
                self.tx_changed(old, status, cx);
                cx.notify();
            }
            SessionEvent::TransactionLost { pending } => {
                self.print(
                    ContentKind::Error,
                    &format!("The transaction was lost with {pending} pending change(s)"),
                    cx,
                );
            }
            SessionEvent::Page {
                result,
                first_row,
                rows,
                has_more,
            } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.result == Some(result)) {
                    tab.grid
                        .update(cx, |g, cx| g.show_page(rows, first_row, has_more, cx));
                }
            }
            SessionEvent::ResultClosed { result } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.result == Some(result)) {
                    tab.grid.update(cx, |g, cx| g.result_closed(cx));
                }
            }
            SessionEvent::CommandFailed { command, error } => {
                self.command_failed(command);
                self.print(ContentKind::Error, &error.message, cx)
            }
            SessionEvent::CommandDone { command } => self.command_done(command, cx),
            SessionEvent::Closed => {}
        }
    }

    fn on_job(&mut self, job: JobId, event: JobEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.running != Some(job) {
            return;
        }
        match event {
            JobEvent::Started { range, text, .. } => {
                let prompt = self.prompt();
                self.print(
                    ContentKind::System,
                    &format!("{prompt}{}", one_line(&text)),
                    cx,
                );
                self.mark_started(range.clone(), cx);
                self.current = Some(Current {
                    text,
                    range,
                    rows: None,
                    affected: None,
                });
            }
            JobEvent::ResultOpened {
                result,
                columns,
                rows,
                has_more,
                ..
            } => {
                if let Some(current) = self.current.as_mut() {
                    current.rows = Some(rows.len());
                }
                let text = self
                    .current
                    .as_ref()
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                let title = tab_title(&text, &columns, self.database.as_deref(), self.dialect());
                self.show_result(result, title, columns, rows, has_more, window, cx);
            }
            JobEvent::Affected { rows, .. } => {
                if let Some(current) = self.current.as_mut() {
                    current.affected = Some(rows);
                }
            }
            JobEvent::Finished { timing, .. } => self.finished(timing, cx),
            JobEvent::Failed { error, timing, .. } => self.failed(error, timing, cx),
            JobEvent::Done { outcome } => {
                self.running = None;
                self.current = None;
                if outcome == JobOutcome::Cancelled {
                    self.clear_mark(cx);
                    self.print(ContentKind::System, "Cancelled", cx);
                }
                cx.notify();
            }
        }
    }

    fn finished(&mut self, timing: Timing, cx: &mut Context<Self>) {
        let ms = |d: std::time::Duration| d.as_millis();
        let total = ms(timing.total());
        self.last_ms = Some(total as u64);
        let current = self.current.take();
        let text = match current {
            Some(Current {
                rows: Some(rows), ..
            }) => format!(
                "{} {} retrieved starting from 1 in {total} ms (execution: {} ms, fetching: {} ms)",
                rows,
                if rows == 1 { "row" } else { "rows" },
                ms(timing.execute),
                ms(timing.fetch),
            ),
            Some(Current {
                affected: Some(n), ..
            }) if n > 0 => format!(
                "{n} {} affected in {total} ms",
                if n == 1 { "row" } else { "rows" }
            ),
            _ => format!("completed in {total} ms"),
        };
        self.print(ContentKind::System, &text, cx);
        self.mark_done(true, total as u64, cx);
    }

    fn failed(&mut self, error: StatementError, timing: Timing, cx: &mut Context<Self>) {
        self.last_ms = Some(timing.total().as_millis() as u64);
        self.mark_done(false, timing.total().as_millis() as u64, cx);
        let start = self.current.take().map(|c| c.range.start).unwrap_or(0);
        let mut text = match &error.error.code {
            Some(code) => format!("[{code}] {}", error.error.message),
            None => error.error.message.clone(),
        };
        if let Some(position) = error.error.position {
            text.push_str(&format!("\n  Position: {}", position + 1));
        } else if error.position > start {
            text.push_str(&format!("\n  Offset: {}", error.position - start + 1));
        }
        self.print(ContentKind::Error, &text, cx);
    }

    // ---- marks in the editor ------------------------------------------------------

    fn editor(&self, cx: &App) -> Option<Entity<Editor>> {
        self.workbench
            .upgrade()
            .and_then(|wb| wb.read(cx).editor_for(&self.path))
    }

    /// The statement that starts running is boxed in the editor, the marks
    /// of the one before are gone.
    fn mark_started(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.clear_mark(cx);
        let Some(editor) = self.editor(cx) else {
            return;
        };
        let text = editor.read(cx).text();
        let range = byte_to_char(&text, range.start)..byte_to_char(&text, range.end);
        let subscription = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            this.on_editor_event(event, cx)
        });
        editor.update(cx, |e, cx| {
            e.set_range_outline(Some((range.clone(), UiRole::Accent)), cx)
        });
        self.mark = Some(Mark {
            range,
            editor: editor.downgrade(),
            _subscription: subscription,
        });
    }

    /// The statement ended: the check mark (or the error mark) in the
    /// gutter of its first line and the time after its last.
    fn mark_done(&mut self, ok: bool, ms: u64, cx: &mut Context<Self>) {
        let Some(mark) = &self.mark else {
            return;
        };
        let Some(editor) = mark.editor.upgrade() else {
            return;
        };
        let range = mark.range.clone();
        let show_time = crate::query_settings::get(cx).show_execution_time;
        editor.update(cx, |e, cx| {
            let first = e.buffer().char_to_line(range.start);
            let last = e.buffer().char_to_line(range.end.max(range.start));
            let icon = if ok {
                crate::icons::STATEMENT_OK
            } else {
                crate::icons::ERROR
            };
            let mut decoration = Decoration::default().lane_icon(icon);
            if show_time {
                decoration = decoration.after_text(
                    vec![TextPiece::new(format!("{ms} ms"), TextRole::Comment)],
                    false,
                );
            }
            e.set_decorations(MARK_GROUP, vec![(first..last + 1, decoration)], cx);
        });
    }

    fn clear_mark(&mut self, cx: &mut Context<Self>) {
        if let Some(mark) = self.mark.take()
            && let Some(editor) = mark.editor.upgrade()
        {
            editor.update(cx, |e, cx| {
                e.set_range_outline(None, cx);
                e.clear_decorations(MARK_GROUP, cx);
            });
        }
    }

    /// An edit of the statement removes its marks, an edit before it moves
    /// them.
    fn on_editor_event(&mut self, event: &EditorEvent, cx: &mut Context<Self>) {
        let Some(mark) = &mut self.mark else {
            return;
        };
        let edits = match event {
            EditorEvent::Edited(edits) => edits,
            EditorEvent::Reloaded => {
                self.clear_mark(cx);
                return;
            }
            _ => return,
        };
        let mut range = mark.range.clone();
        for e in edits {
            match shift_for_edit(&range, e.start, e.old_end, e.new_end) {
                Some(moved) => range = moved,
                None => {
                    self.clear_mark(cx);
                    return;
                }
            }
        }
        if range != mark.range {
            mark.range = range.clone();
            if let Some(editor) = mark.editor.upgrade() {
                editor.update(cx, |e, cx| {
                    e.set_range_outline(Some((range, UiRole::Accent)), cx)
                });
            }
        }
    }

    // ---- results ----------------------------------------------------------------

    /// Shows a statement's result: in the first tab that is neither pinned
    /// nor renamed, else in a new one.
    #[allow(clippy::too_many_arguments)]
    fn show_result(
        &mut self,
        result: ResultId,
        title: String,
        columns: Vec<db_core::Column>,
        rows: Vec<db_core::Row>,
        has_more: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.clone();
        let fetch: crate::grid::FetchNext = std::rc::Rc::new(move |_| session.fetch(result, None));
        let target = GridTarget {
            source: self.source.clone(),
            backend: self.backend,
            handle: self.handle.clone(),
            table: db_session::TableRef::new(None, &title),
            columns: Vec::new(),
        };
        let grid = cx.new(|cx| TableGrid::result(target, columns, Some(fetch), window, cx));
        grid.update(cx, |g, cx| g.show_page(rows, 0, has_more, cx));
        // The result shown is the grid the grid commands go to.
        crate::grid::touch(self.window, &grid, cx);
        let tab = ResultTab {
            title: title.into(),
            pinned: false,
            renamed: false,
            grid,
            result: Some(result),
        };
        let reuse = self.tabs.iter().position(|t| !t.pinned && !t.renamed);
        let index = match reuse {
            Some(ix) => {
                self.tabs[ix] = tab;
                ix
            }
            None => {
                self.tabs.push(tab);
                self.tabs.len() - 1
            }
        };
        self.pane = Pane::Result(index);
        cx.notify();
    }

    pub fn select_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.pane = match pane {
            Pane::Result(i) if i >= self.tabs.len() => Pane::Output,
            other => other,
        };
        cx.notify();
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.pane = match self.pane {
            Pane::Result(i) if i == index => Pane::Output,
            Pane::Result(i) if i > index => Pane::Result(i - 1),
            other => other,
        };
        cx.notify();
    }

    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "path": self.path.display().to_string(),
            "file": self.kind == ConsoleKind::File,
            "label": self.label(),
            "source": self.source,
            "schema": self.schema,
            "running": self.running.is_some(),
            "soft_wrap": self.soft_wrap,
            "tx": self.tx_json(),
            "safety": self.safety_json(),
            "output": self.lines,
            "pane": match self.pane {
                Pane::Output => json!("output"),
                Pane::Result(i) => json!(i),
            },
            "tabs": self.tabs.iter().map(|t| json!({
                "title": t.title.to_string(),
                "pinned": t.pinned,
                "grid": t.grid.read(cx).state_json(cx),
            })).collect::<Vec<_>>(),
        })
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The char offset of a byte offset (clamped to the text).
fn byte_to_char(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

/// Where a statement's char range is after an edit replacing
/// `start..old_end` with text ending at `new_end`; `None` when the edit
/// touches it.
pub(crate) fn shift_for_edit(
    range: &Range<usize>,
    start: usize,
    old_end: usize,
    new_end: usize,
) -> Option<Range<usize>> {
    if old_end < range.start {
        let moved = |at: usize| at + new_end - old_end;
        Some(moved(range.start)..moved(range.end))
    } else if start > range.end {
        Some(range.clone())
    } else {
        None
    }
}

fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

/// The name of a result tab: the table a plain query reads, with its
/// database (`pyoxide.demo.customers`), else the first column and its
/// type (`a:integer`).
pub(crate) fn tab_title(
    text: &str,
    columns: &[db_core::Column],
    database: Option<&str>,
    dialect: Dialect,
) -> String {
    let tokens: Vec<_> = sql_syntax::lex(text, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .collect();
    let from = tokens
        .iter()
        .position(|t| t.kind == TokenKind::Word && t.text(text).eq_ignore_ascii_case("from"));
    if let Some(from) = from {
        let mut parts: Vec<String> = Vec::new();
        let mut ix = from + 1;
        while let Some(t) = tokens.get(ix) {
            if !matches!(t.kind, TokenKind::Word | TokenKind::QuotedIdent) {
                break;
            }
            parts.push(
                dialect
                    .unquote(t.text(text))
                    .unwrap_or_else(|| t.text(text).to_owned()),
            );
            if tokens.get(ix + 1).is_some_and(|d| d.kind == TokenKind::Dot) {
                ix += 2;
            } else {
                break;
            }
        }
        if !parts.is_empty() {
            let mut full: Vec<String> = database.map(str::to_owned).into_iter().collect();
            full.extend(parts);
            return full.join(".");
        }
    }
    match columns.first() {
        Some(c) if columns.len() == 1 => format!("{}:{}", c.name, c.type_name.to_lowercase()),
        _ => "Result".to_owned(),
    }
}
