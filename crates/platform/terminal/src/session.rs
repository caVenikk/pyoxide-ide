//! A terminal session: a shell on a pseudo-terminal, the emulator state it
//! writes into, and the calls of the view (input, resize, snapshot,
//! selection, search, scrolling, stopping).

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point as GridPoint};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
use process::ExitStatus;

use crate::input::{self, Key, Modifiers, MouseAction};
use crate::io_loop::{self, LoopSender, Msg};
use crate::prescan::MarkKind;
use crate::running::CommandProbe;
use crate::snapshot::{
    self, MAX_VISIBLE_MATCHES, Modes, Point, Rgb, Snapshot, Span, clip_span, to_grid,
};
use crate::start::ShellCommand;

/// Lines of scrollback by default (ADR 0014).
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// The size of the session's screen in cells (and of a cell in pixels,
/// for programs that ask).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        }
    }
}

impl TerminalSize {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self {
            columns,
            rows,
            ..Self::default()
        }
    }

    /// The emulator's minimum: 2 columns, 1 row.
    fn clamped(self) -> Self {
        Self {
            columns: self.columns.max(term::MIN_COLUMNS as u16),
            rows: self.rows.max(term::MIN_SCREEN_LINES as u16),
            ..self
        }
    }

    pub(crate) fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.columns,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.columns as usize
    }
}

/// How to start a session: see [`crate::plan_session`] for the start rules.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub command: ShellCommand,
    pub working_directory: PathBuf,
    /// Variables set over the inherited environment.
    pub env: Vec<(String, String)>,
    pub size: TerminalSize,
    /// Lines of scrollback; 0: [`DEFAULT_SCROLLBACK`].
    pub scrollback_lines: usize,
}

/// Why a session did not start.
#[derive(Debug)]
pub enum SpawnError {
    /// Windows older than 10 1809 (build 17763): no ConPTY.
    NoPseudoConsole { build: u32 },
    /// The working directory is not a folder.
    WorkingDirectory(PathBuf),
    /// The pseudo-terminal or the process could not be created.
    Io(std::io::Error),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpawnError::NoPseudoConsole { build } => write!(
                f,
                "The terminal needs Windows 10 version 1809 or later (this is build {build})"
            ),
            SpawnError::WorkingDirectory(path) => {
                write!(f, "The folder {} does not exist", path.display())
            }
            SpawnError::Io(e) => write!(f, "Cannot start the shell: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {}

/// A shell integration mark (OSC 133) with where the cursor was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellMark {
    pub kind: MarkKind,
    /// The cursor's row on the screen (0: top).
    pub cursor_line: usize,
    /// Lines in the scrollback at that moment.
    pub history_size: usize,
}

/// What a session tells the view, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// The screen changed: take a new snapshot. Sent once until the next
    /// snapshot is taken.
    Wakeup,
    /// The program set (or reset) the window title (OSC 0 / 2).
    Title(Option<String>),
    Bell,
    /// The program put text on the clipboard (OSC 52).
    ClipboardStore(String),
    /// The shell reported its working directory (OSC 7).
    WorkingDirectory(PathBuf),
    /// A prompt or command mark (OSC 133).
    Mark(ShellMark),
    /// A Pyoxide integration event ([`crate::OWN_OSC`]) with its payload.
    Integration(String),
    /// The shell exited; the last event.
    Exited(ExitStatus),
}

/// The colours the session answers colour queries with (OSC 4, 10, 11,
/// 12 with `?`), set from the theme by the view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// ANSI colours 0 to 15.
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

impl Default for Palette {
    /// xterm's colours on a light background.
    fn default() -> Self {
        let c = Rgb::new;
        Self {
            ansi: [
                c(0x00, 0x00, 0x00),
                c(0xcd, 0x00, 0x00),
                c(0x00, 0xcd, 0x00),
                c(0xcd, 0xcd, 0x00),
                c(0x00, 0x00, 0xee),
                c(0xcd, 0x00, 0xcd),
                c(0x00, 0xcd, 0xcd),
                c(0xe5, 0xe5, 0xe5),
                c(0x7f, 0x7f, 0x7f),
                c(0xff, 0x00, 0x00),
                c(0x00, 0xff, 0x00),
                c(0xff, 0xff, 0x00),
                c(0x5c, 0x5c, 0xff),
                c(0xff, 0x00, 0xff),
                c(0x00, 0xff, 0xff),
                c(0xff, 0xff, 0xff),
            ],
            foreground: c(0x00, 0x00, 0x00),
            background: c(0xff, 0xff, 0xff),
            cursor: c(0x00, 0x00, 0x00),
        }
    }
}

impl Palette {
    /// The colour of an emulator colour index: 0 to 15 from the theme,
    /// 16 to 255 the standard cube and grey ramp, then the defaults.
    pub fn color(&self, index: usize) -> Option<Rgb> {
        Some(match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let i = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                Rgb::new(level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            232..=255 => {
                let v = (8 + (index - 232) * 10) as u8;
                Rgb::new(v, v, v)
            }
            256 => self.foreground,
            257 => self.background,
            258 => self.cursor,
            _ => return None,
        })
    }
}

/// Search options of the find bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub match_case: bool,
    pub regex: bool,
}

impl SearchQuery {
    /// The regular expression for the emulator's search: the text escaped
    /// unless it is a regular expression, with the case flag up front.
    pub fn pattern(&self) -> String {
        let body = if self.regex {
            self.text.clone()
        } else {
            escape_regex(&self.text)
        };
        let flag = if self.match_case { "(?-i)" } else { "(?i)" };
        format!("{flag}{body}")
    }
}

fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The query is not a valid regular expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchError(pub String);

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SearchError {}

/// The number of matches and the position of the current one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchCount {
    /// Matches counted (at most the limit given).
    pub total: usize,
    /// 0-based index of the current match among them.
    pub current: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    /// Towards the end of the output.
    Next,
    /// Towards the start of the output.
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character by character (drag).
    Simple,
    /// A rectangle (Shift+Alt drag).
    Block,
    /// Whole words (double click).
    Word,
    /// Whole lines (triple click).
    Line,
}

/// Which half of a cell the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Scrolling the view through the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Lines up (positive, towards older output) or down.
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Shared with the I/O loop and the emulator's event listener.
pub(crate) struct Notifier {
    events: flume::Sender<SessionEvent>,
    wakeup_pending: AtomicBool,
    pub(crate) info: Mutex<Info>,
    window_size: Mutex<WindowSize>,
    palette: Mutex<Palette>,
    loop_tx: LoopSender,
}

#[derive(Default)]
pub(crate) struct Info {
    pub(crate) title: Option<String>,
    pub(crate) working_directory: Option<PathBuf>,
    pub(crate) exit: Option<ExitStatus>,
}

impl Notifier {
    /// Sends [`SessionEvent::Wakeup`] unless one is pending.
    pub(crate) fn wake(&self) {
        if !self.wakeup_pending.swap(true, Ordering::AcqRel) {
            let _ = self.events.send(SessionEvent::Wakeup);
        }
    }

    pub(crate) fn send(&self, event: SessionEvent) {
        let _ = self.events.send(event);
    }

    pub(crate) fn info(&self) -> MutexGuard<'_, Info> {
        self.info.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The emulator's event listener.
pub(crate) struct Listener(Arc<Notifier>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let n = &self.0;
        match event {
            Event::Title(title) => {
                n.info().title = Some(title.clone());
                n.send(SessionEvent::Title(Some(title)));
            }
            Event::ResetTitle => {
                n.info().title = None;
                n.send(SessionEvent::Title(None));
            }
            Event::ClipboardStore(_, text) => n.send(SessionEvent::ClipboardStore(text)),
            Event::PtyWrite(text) => n.loop_tx.send(Msg::Input(text.into_bytes().into())),
            Event::TextAreaSizeRequest(reply) => {
                let size = *n.window_size.lock().unwrap_or_else(|e| e.into_inner());
                n.loop_tx.send(Msg::Input(reply(size).into_bytes().into()));
            }
            Event::ColorRequest(index, reply) => {
                let color = n
                    .palette
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .color(index);
                if let Some(c) = color {
                    let rgb = alacritty_terminal::vte::ansi::Rgb {
                        r: c.r,
                        g: c.g,
                        b: c.b,
                    };
                    n.loop_tx.send(Msg::Input(reply(rgb).into_bytes().into()));
                }
            }
            Event::Bell => n.send(SessionEvent::Bell),
            Event::Wakeup | Event::CursorBlinkingChange => n.wake(),
            Event::MouseCursorDirty
            | Event::ClipboardLoad(..)
            | Event::Exit
            | Event::ChildExit(_) => {}
        }
    }
}

/// The emulator and the search, under the terminal's lock.
pub(crate) struct State {
    pub(crate) term: Term<Listener>,
    search: Option<SearchState>,
}

struct SearchState {
    regex: RegexSearch,
    current: Option<Match>,
}

pub(crate) struct Shared {
    pub(crate) state: FairMutex<State>,
    pub(crate) notifier: Arc<Notifier>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Copies the screen of a session from any thread
/// ([`Session::screen`]); it keeps the emulator state alive, not the shell.
#[derive(Clone)]
pub struct Screen {
    shared: Arc<Shared>,
}

impl fmt::Debug for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Screen").finish_non_exhaustive()
    }
}

impl Screen {
    /// [`Session::snapshot_into`].
    pub fn snapshot_into(&self, out: &mut Snapshot) {
        fill_snapshot(&self.shared, out);
    }
}

/// Copies the visible state under the terminal's lock.
fn fill_snapshot(shared: &Shared, out: &mut Snapshot) {
    // Output after this point sends a new wakeup.
    shared
        .notifier
        .wakeup_pending
        .store(false, Ordering::Release);
    let mut guard = shared.state.lock();
    let State { term, search } = &mut *guard;
    snapshot::fill(term, out);
    if let Some(search) = search {
        visible_matches(term, &mut search.regex, &mut out.matches);
        out.current_match = search
            .current
            .as_ref()
            .and_then(|m| clip_span(term, *m.start(), *m.end(), false));
    }
}

/// A running (or ended) shell session. Dropping it stops the shell: hang
/// up, then kill after a grace period, on a background thread.
pub struct Session {
    shared: Arc<Shared>,
    loop_tx: LoopSender,
    pid: u32,
    id: u64,
    probe: CommandProbe,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts the shell and the reader thread. Events arrive on the
    /// returned channel; [`SessionEvent::Exited`] is the last one.
    pub fn spawn(
        options: SessionOptions,
    ) -> Result<(Session, flume::Receiver<SessionEvent>), SpawnError> {
        if let Some(build) = crate::conpty::unsupported_windows_build() {
            return Err(SpawnError::NoPseudoConsole { build });
        }
        if !options.working_directory.is_dir() {
            return Err(SpawnError::WorkingDirectory(options.working_directory));
        }
        let size = options.size.clamped();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (program, args) = crate::start::pty_program(&options.command, &options.env);
        let tty_options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: Some(options.working_directory.clone()),
            drain_on_exit: true,
            env: options.env.iter().cloned().collect::<HashMap<_, _>>(),
            #[cfg(windows)]
            escape_args: true,
        };
        // `tty::new` asserts on some failures (ConPTY creation): report
        // them as errors instead of unwinding into the caller.
        let pty = std::panic::catch_unwind(|| tty::new(&tty_options, size.window_size(), 0))
            .map_err(|_| {
                SpawnError::Io(std::io::Error::other(
                    "the pseudo-terminal could not be created",
                ))
            })?
            .map_err(SpawnError::Io)?;
        let pid = io_loop::child_pid(&pty);

        let (events_tx, events_rx) = flume::unbounded();
        let (loop_tx, loop_rx) = io_loop::channel().map_err(SpawnError::Io)?;
        let notifier = Arc::new(Notifier {
            events: events_tx,
            wakeup_pending: AtomicBool::new(false),
            info: Mutex::new(Info::default()),
            window_size: Mutex::new(size.window_size()),
            palette: Mutex::new(Palette::default()),
            loop_tx: loop_tx.clone(),
        });
        let scrollback = match options.scrollback_lines {
            0 => DEFAULT_SCROLLBACK,
            n => n,
        };
        let config = term::Config {
            scrolling_history: scrollback,
            // The reference's default: a block that blinks (the editor's
            // caret blinks by default); a program can ask for another.
            default_cursor_style: CursorStyle {
                shape: CursorShape::Block,
                blinking: true,
            },
            ..term::Config::default()
        };
        let term = Term::new(config, &size, Listener(notifier.clone()));
        let shared = Arc::new(Shared {
            state: FairMutex::new(State { term, search: None }),
            notifier,
        });
        io_loop::start(pty, shared.clone(), loop_tx.clone(), loop_rx, id)
            .map_err(SpawnError::Io)?;
        Ok((
            Session {
                shared,
                loop_tx,
                pid,
                id,
                probe: CommandProbe::new(pid, &options.command.program),
            },
            events_rx,
        ))
    }

    /// The shell's process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The question "does a command run in the shell?" for a background
    /// thread: [`CommandProbe::has_running_commands`].
    pub fn command_probe(&self) -> CommandProbe {
        self.probe.clone()
    }

    /// Whether a command runs in the shell (it has a child process).
    /// Reads the process table: call it off the UI thread.
    pub fn has_running_commands(&self) -> bool {
        self.probe.has_running_commands()
    }

    /// A number unique among the sessions of this process.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Sends bytes to the shell as they are.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            self.loop_tx.send(Msg::Input(bytes));
        }
    }

    /// Sends typed text (IME commits, composed characters) and scrolls to
    /// the bottom.
    pub fn write_text(&self, text: &str) {
        self.scroll_to_bottom_for_input();
        self.write(text.as_bytes().to_vec());
    }

    /// Encodes a key for the current modes and sends it; `false` when the
    /// key sends nothing.
    pub fn send_key(&self, key: Key, mods: Modifiers) -> bool {
        let modes = self.modes();
        match input::encode_key(key, mods, &modes) {
            Some(bytes) => {
                self.scroll_to_bottom_for_input();
                self.write(bytes);
                true
            }
            None => false,
        }
    }

    /// Pastes text: bracketed when the program asked for it.
    pub fn paste(&self, text: &str) {
        let modes = self.modes();
        self.scroll_to_bottom_for_input();
        self.write(input::encode_paste(text, modes.bracketed_paste));
    }

    /// Reports a mouse event when the program asked for mouse reporting;
    /// `false` when it did not (the view selects text instead).
    pub fn send_mouse(&self, action: MouseAction, point: Point, mods: Modifiers) -> bool {
        let modes = self.modes();
        match input::encode_mouse(action, point.column, point.line, mods, &modes) {
            Some(bytes) => {
                self.write(bytes);
                true
            }
            None => false,
        }
    }

    /// The wheel on the alternate screen without mouse reporting: cursor
    /// keys. `false` when the wheel scrolls the history instead.
    pub fn send_alternate_scroll(&self, lines: i32) -> bool {
        match input::encode_alternate_scroll(lines, &self.modes()) {
            Some(bytes) => {
                self.write(bytes);
                true
            }
            None => false,
        }
    }

    /// The view gained or lost the focus: a focus report when asked for.
    pub fn set_focused(&self, focused: bool) {
        let modes = {
            let mut state = self.shared.state.lock();
            state.term.is_focused = focused;
            Modes::from_term(*state.term.mode())
        };
        if let Some(bytes) = input::encode_focus(focused, &modes) {
            self.write(bytes);
        }
    }

    /// The current modes (for the view's key and mouse handling).
    pub fn modes(&self) -> Modes {
        Modes::from_term(*self.shared.state.lock().term.mode())
    }

    /// Resizes the screen and the pseudo-terminal.
    pub fn resize(&self, size: TerminalSize) {
        let size = size.clamped();
        {
            let mut state = self.shared.state.lock();
            if state.term.columns() == size.columns as usize
                && state.term.screen_lines() == size.rows as usize
            {
                return;
            }
            state.term.resize(size);
        }
        *self
            .shared
            .notifier
            .window_size
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = size.window_size();
        self.loop_tx.send(Msg::Resize(size.window_size()));
        self.shared.notifier.wake();
    }

    /// The colours for answering colour queries.
    pub fn set_palette(&self, palette: Palette) {
        *self
            .shared
            .notifier
            .palette
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = palette;
    }

    /// The visible state now.
    pub fn snapshot(&self) -> Snapshot {
        let mut out = Snapshot::default();
        self.snapshot_into(&mut out);
        out
    }

    /// [`Session::snapshot`] into an existing snapshot, reusing its
    /// buffers (once per frame).
    pub fn snapshot_into(&self, out: &mut Snapshot) {
        fill_snapshot(&self.shared, out);
    }

    /// A handle that copies the screen from another thread, so the view
    /// never waits for the terminal's lock on the UI thread.
    pub fn screen(&self) -> Screen {
        Screen {
            shared: self.shared.clone(),
        }
    }

    /// The window title the program set, if any.
    pub fn title(&self) -> Option<String> {
        self.shared.notifier.info().title.clone()
    }

    /// The working directory the shell last reported (OSC 7).
    pub fn working_directory(&self) -> Option<PathBuf> {
        self.shared.notifier.info().working_directory.clone()
    }

    /// How the shell ended, once it has.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.shared.notifier.info().exit
    }

    pub fn scroll(&self, scroll: Scroll) {
        let scroll = match scroll {
            Scroll::Lines(n) => GridScroll::Delta(n),
            Scroll::PageUp => GridScroll::PageUp,
            Scroll::PageDown => GridScroll::PageDown,
            Scroll::Top => GridScroll::Top,
            Scroll::Bottom => GridScroll::Bottom,
        };
        self.shared.state.lock().term.scroll_display(scroll);
        self.shared.notifier.wake();
    }

    fn scroll_to_bottom_for_input(&self) {
        let mut state = self.shared.state.lock();
        if state.term.grid().display_offset() != 0 {
            state.term.scroll_display(GridScroll::Bottom);
            drop(state);
            self.shared.notifier.wake();
        }
    }

    /// Starts a selection at a visible cell.
    pub fn start_selection(&self, kind: SelectionKind, point: Point, side: Side) {
        let mut state = self.shared.state.lock();
        let at = to_grid(&state.term, point);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        state.term.selection = Some(Selection::new(ty, at, grid_side(side)));
        drop(state);
        self.shared.notifier.wake();
    }

    /// Moves the end of the selection to a visible cell.
    pub fn update_selection(&self, point: Point, side: Side) {
        let mut state = self.shared.state.lock();
        let at = to_grid(&state.term, point);
        if let Some(selection) = &mut state.term.selection {
            selection.update(at, grid_side(side));
        }
        drop(state);
        self.shared.notifier.wake();
    }

    pub fn clear_selection(&self) {
        self.shared.state.lock().term.selection = None;
        self.shared.notifier.wake();
    }

    /// Selects all output, the scrollback included.
    pub fn select_all(&self) {
        let mut state = self.shared.state.lock();
        let term = &mut state.term;
        let start = GridPoint::new(term.topmost_line(), Column(0));
        let end = GridPoint::new(term.bottommost_line(), term.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, grid_side(Side::Left));
        selection.update(end, grid_side(Side::Right));
        term.selection = Some(selection);
        drop(state);
        self.shared.notifier.wake();
    }

    /// Whether a selection was started, empty or not: a click sets the
    /// anchor that Shift+click extends.
    pub fn has_selection(&self) -> bool {
        self.shared.state.lock().term.selection.is_some()
    }

    /// The selected text, if anything is selected.
    pub fn selection_text(&self) -> Option<String> {
        self.shared
            .state
            .lock()
            .term
            .selection_to_string()
            .filter(|s| !s.is_empty())
    }

    /// Sets (or with `None` or an empty text, clears) the search query.
    pub fn set_search(&self, query: Option<&SearchQuery>) -> Result<(), SearchError> {
        let search = match query.filter(|q| !q.text.is_empty()) {
            Some(query) => Some(SearchState {
                regex: RegexSearch::new(&query.pattern())
                    .map_err(|e| SearchError(e.to_string()))?,
                current: None,
            }),
            None => None,
        };
        self.shared.state.lock().search = search;
        self.shared.notifier.wake();
        Ok(())
    }

    /// Moves to the next or previous match (around the ends), scrolls it
    /// into view and returns it. From no current match, `Previous` finds
    /// the last match of the output and `Next` the first.
    pub fn find(&self, direction: SearchDirection) -> Option<Span> {
        let mut guard = self.shared.state.lock();
        let State { term, search } = &mut *guard;
        let search = search.as_mut()?;
        let (origin, dir, side) = match (&search.current, direction) {
            (Some(m), SearchDirection::Next) => (
                m.end().add(term, Boundary::None, 1),
                Direction::Right,
                Direction::Left,
            ),
            (Some(m), SearchDirection::Previous) => (
                m.start().sub(term, Boundary::None, 1),
                Direction::Left,
                Direction::Right,
            ),
            (None, SearchDirection::Next) => (
                GridPoint::new(term.topmost_line(), Column(0)),
                Direction::Right,
                Direction::Left,
            ),
            (None, SearchDirection::Previous) => (
                GridPoint::new(term.bottommost_line(), term.last_column()),
                Direction::Left,
                Direction::Right,
            ),
        };
        let found = term.search_next(&mut search.regex, origin, dir, side, None)?;
        term.scroll_to_point(*found.start());
        let span = clip_span(term, *found.start(), *found.end(), false);
        search.current = Some(found);
        drop(guard);
        self.shared.notifier.wake();
        span
    }

    /// Counts the matches in the whole output, up to `limit`. Walks the
    /// scrollback under the terminal's lock: call it when the query or
    /// the output changed, not per frame.
    pub fn count_matches(&self, limit: usize) -> MatchCount {
        let mut guard = self.shared.state.lock();
        let State { term, search } = &mut *guard;
        let Some(search) = search.as_mut() else {
            return MatchCount::default();
        };
        let start = GridPoint::new(term.topmost_line(), Column(0));
        let end = GridPoint::new(term.bottommost_line(), term.last_column());
        let mut count = MatchCount::default();
        for m in RegexIter::new(start, end, Direction::Right, term, &mut search.regex) {
            if search.current.as_ref() == Some(&m) {
                count.current = Some(count.total);
            }
            count.total += 1;
            if count.total >= limit {
                break;
            }
        }
        count
    }

    /// Clear Terminal: drops the scrollback and the screen above the
    /// cursor's line, which moves to the top (the prompt stays). Does
    /// nothing on the alternate screen.
    pub fn clear(&self) {
        let mut guard = self.shared.state.lock();
        let State { term, search } = &mut *guard;
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let rows = term.screen_lines();
        let line = term.grid().cursor.point.line.0.max(0) as usize;
        if line > 0 {
            term.grid_mut()
                .scroll_up::<alacritty_terminal::vte::ansi::Color>(
                    &(Line(0)..Line(rows as i32)),
                    line,
                );
            term.grid_mut().cursor.point.line = Line(0);
        }
        term.grid_mut().clear_history();
        term.selection = None;
        if let Some(search) = search {
            search.current = None;
        }
        drop(guard);
        self.shared.notifier.wake();
    }

    /// Hangs up the shell (`SIGHUP`; on Windows the process is ended).
    pub fn terminate(&self) {
        self.loop_tx.send(Msg::Terminate);
    }

    /// Kills the shell (`SIGKILL`; `TerminateProcess` on Windows).
    pub fn kill(&self) {
        self.loop_tx.send(Msg::Kill);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.loop_tx.send(Msg::Shutdown);
    }
}

fn grid_side(side: Side) -> Direction {
    match side {
        Side::Left => Direction::Left,
        Side::Right => Direction::Right,
    }
}

/// The matches around the viewport (a screen above and below, for matches
/// that cross its edges), clipped to it.
fn visible_matches<T>(term: &Term<T>, regex: &mut RegexSearch, out: &mut Vec<Span>) {
    out.clear();
    let offset = term.grid().display_offset() as i32;
    let rows = term.screen_lines() as i32;
    let start = GridPoint::new(Line(-offset - rows).max(term.topmost_line()), Column(0));
    let end = GridPoint::new(
        Line(-offset + 2 * rows - 1).min(term.bottommost_line()),
        term.last_column(),
    );
    for (seen, m) in RegexIter::new(start, end, Direction::Right, term, regex).enumerate() {
        if seen >= 4 * MAX_VISIBLE_MATCHES || out.len() >= MAX_VISIBLE_MATCHES {
            break;
        }
        if let Some(span) = clip_span(term, *m.start(), *m.end(), false) {
            out.push(span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_patterns() {
        let q = |text: &str, match_case, regex| SearchQuery {
            text: text.into(),
            match_case,
            regex,
        };
        assert_eq!(q("a.b", false, false).pattern(), "(?i)a\\.b");
        assert_eq!(q("F.O", true, true).pattern(), "(?-i)F.O");
        assert!(RegexSearch::new(&q("x(", false, false).pattern()).is_ok());
        assert!(RegexSearch::new(&q("x(", false, true).pattern()).is_err());
    }

    #[test]
    fn palette_cube_and_ramp() {
        let p = Palette::default();
        assert_eq!(p.color(1), Some(p.ansi[1]));
        assert_eq!(p.color(16), Some(Rgb::new(0, 0, 0)));
        assert_eq!(p.color(21), Some(Rgb::new(0, 0, 255)));
        assert_eq!(p.color(196), Some(Rgb::new(255, 0, 0)));
        assert_eq!(p.color(232), Some(Rgb::new(8, 8, 8)));
        assert_eq!(p.color(255), Some(Rgb::new(238, 238, 238)));
        assert_eq!(p.color(257), Some(p.background));
        assert_eq!(p.color(300), None);
    }
}
