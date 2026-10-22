//! The session view: one terminal session in a focusable view. After the
//! session says the screen changed it copies the screen on a background
//! thread, at most once per frame, and asks its owner to draw the copy at
//! once ([`SessionViewEvent::Redraw`]); while output floods it copies ten
//! times a second and repaints at the start of the next frame. It passes
//! keys, typed and composed text, pastes, mouse
//! reports and resizes to the session, selects text with the mouse
//! (drag, double click for a word, triple click for a line, Alt or
//! Shift+Alt drag for a rectangle, Shift+click to extend), scrolls the
//! history with the wheel and the scrollbar, and handles the Terminal
//! commands (Copy, Paste, Select All, Clear, Line and Page Up/Down).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, Modifiers as GpuiModifiers, MouseButton as GpuiButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point as GpuiPoint, Render,
    ScrollWheelEvent, Size, Task, Window, div, prelude::*, px,
};
use serde_json::{Value, json};
use terminal::{
    ExitStatus, MarkKind, Modifiers, MouseAction, MouseButton, MouseMode, Point, Scroll,
    SelectionKind, Session, SessionEvent, SessionOptions, Side, Snapshot, TerminalSize,
};
use themes::UiRole;
use ui::theme::ThemeExt;

use crate::colors::{Resolver, TerminalColors};
use crate::commands::{Clear, Copy, LineDown, LineUp, PageDown, PageUp, Paste, SelectAll};
use crate::element::TerminalElement;
use crate::keys;

/// The key context of the view (keymaps' `Terminal` sections).
pub const KEY_CONTEXT: &str = "Terminal";
/// Shortest time between two copies of the screen: one frame, so output
/// costs at most one repaint per frame.
pub const FRAME: Duration = Duration::from_millis(16);
/// Shortest time between two copies while output floods: the screen is
/// copied and its lines shaped ten times a second, which leaves the
/// frames in between to the rest of the window (the editor being typed
/// in while a build prints).
pub const FLOOD_FRAME: Duration = Duration::from_millis(100);
/// Output that keeps coming this long (each change within
/// [`FLOOD_FRAME`] of the last copy) is a flood. Input and scrolling end
/// it.
pub const FLOOD_AFTER: Duration = Duration::from_millis(200);
/// How long the blinking cursor stays visible or hidden (the editor's).
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// The terminal options of the view (Settings | Tools | Terminal in the
/// reference; the defaults are the reference's).
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalOptions {
    /// "Use Option as Meta key" (macOS).
    pub option_as_meta: bool,
    /// "Enable mouse reporting".
    pub mouse_reporting: bool,
    /// "Copy to clipboard on selection".
    pub copy_on_selection: bool,
    /// "Paste on middle mouse button click".
    pub paste_on_middle_click: bool,
    /// The cursor blinks (the editor's "Caret blinking").
    pub blink: bool,
    /// The wheel sends arrow keys on the alternate screen when the program
    /// does not report the mouse.
    pub alternate_scroll: bool,
    /// Line height as a multiple of the font's (the terminal font's line
    /// spacing).
    pub line_spacing: f32,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            option_as_meta: false,
            mouse_reporting: true,
            copy_on_selection: false,
            paste_on_middle_click: true,
            blink: true,
            alternate_scroll: true,
            line_spacing: 1.0,
        }
    }
}

/// What the view tells its owner (the Terminal tool window).
#[derive(Clone, Debug, PartialEq)]
pub enum SessionViewEvent {
    /// A new copy of the screen is ready (output that does not flood, such
    /// as the echo of a key): the owner draws the window now rather than at
    /// the next display refresh, as after a keystroke in the editor
    /// (the workbench's `draw_now`). The view is already notified.
    Redraw,
    TitleChanged(Option<String>),
    /// A command started or finished in the shell (shell integration).
    CommandRunningChanged(bool),
    WorkingDirectoryChanged(PathBuf),
    Bell,
    Exited(ExitStatus),
}

/// Where the grid was laid out in the last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub bounds: Bounds<Pixels>,
    /// The top left corner of cell (0, 0).
    pub origin: GpuiPoint<Pixels>,
    /// Cell width and line height.
    pub cell: Size<Pixels>,
    pub columns: usize,
    pub rows: usize,
    pub scrollbar: Option<ScrollbarGeometry>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarGeometry {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// Lines in the history above the screen.
    pub history: usize,
}

impl Geometry {
    /// The cell under a position (clamped to the grid) and the side of the
    /// cell it is on.
    pub fn cell_at(&self, position: GpuiPoint<Pixels>) -> (Point, Side) {
        let x = f32::from(position.x - self.origin.x) / f32::from(self.cell.width);
        let y = f32::from(position.y - self.origin.y) / f32::from(self.cell.height);
        let column = (x.max(0.0) as usize).min(self.columns.saturating_sub(1));
        let line = (y.max(0.0) as usize).min(self.rows.saturating_sub(1));
        let side = if x - x.floor() < 0.5 || x < 0.0 {
            Side::Left
        } else {
            Side::Right
        };
        let side = if x >= self.columns as f32 {
            Side::Right
        } else {
            side
        };
        (Point::new(line, column), side)
    }
}

/// What a frame of the view showed (measurements).
#[derive(Clone, Debug)]
pub struct FrameRecord {
    /// When the frame was laid out.
    pub at: Instant,
    /// Snapshots taken up to this frame.
    pub snapshots: u64,
    pub cursor: Option<Point>,
    pub display_offset: usize,
    /// The last visible lines that are not blank, the bottom one first
    /// (at most [`FrameRecord::TAIL`]).
    pub tail: Vec<String>,
    /// When the first change this snapshot shows was reported (the
    /// session's wakeup reached the view).
    pub woke: Option<Instant>,
    /// When the snapshot was copied.
    pub copied: Option<Instant>,
}

impl FrameRecord {
    /// Lines kept in [`FrameRecord::tail`]: a prompt can follow the
    /// output that is looked for.
    pub const TAIL: usize = 4;

    /// Whether one of the last lines is `text`.
    pub fn shows_line(&self, text: &str) -> bool {
        self.tail.iter().any(|l| l == text)
    }
}

/// The mouse gesture in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    None,
    /// Selecting text with the left button.
    Selecting,
    /// A button reported to the program.
    Reporting(MouseButton),
    /// Dragging the scrollbar's thumb, grabbed this far below its top.
    Scrollbar(Pixels),
}

/// A terminal session in a view.
pub struct SessionView {
    session: Option<Session>,
    /// A session to start once the first frame gives its size.
    pending: Option<SessionOptions>,
    /// Text typed before the session started.
    pending_input: String,
    error: Option<String>,
    exit: Option<ExitStatus>,
    title: Option<String>,
    working_directory: Option<PathBuf>,
    /// The shell runs a command: between its integration's marks C (the
    /// command runs) and D or A (it finished, the next prompt).
    command_running: bool,
    focus: FocusHandle,
    options: TerminalOptions,
    pub(crate) snapshot: Snapshot,
    /// The buffers of the next copy (the snapshot shown before).
    spare: Snapshot,
    /// Copy the screen in the next frame, on the UI thread: a new session,
    /// a new size, or the user scrolled or selected, which the frame must
    /// show.
    copy_in_frame: bool,
    /// The session reported a change that is not copied yet.
    changed: bool,
    /// The copy running on a background thread.
    copying: Option<Task<()>>,
    /// Counts copies in a frame: a background copy started before one is
    /// older and dropped.
    copy_epoch: u64,
    /// The last copy is not drawn yet (no new copy until it is).
    unshown: bool,
    /// A repaint is requested for the start of the next frame.
    frame_requested: bool,
    /// Waits out the shortest time between two copies.
    refresh_task: Option<Task<()>>,
    /// When the last copy started.
    last_copy: Option<Instant>,
    /// Since when output keeps coming.
    busy_since: Option<Instant>,
    /// The window the view was last drawn in (its frames repaint it).
    pub(crate) window: AnyWindowHandle,
    pub(crate) resolver: Resolver,
    /// The colours the session answers colour queries with.
    palette: Option<terminal::Palette>,
    pub(crate) geometry: Option<Geometry>,
    size: Option<TerminalSize>,
    gesture: Gesture,
    /// The last cell a motion was reported for.
    last_motion: Option<Point>,
    /// Pixels the wheel moved short of a whole line.
    wheel_remainder: f32,
    pub(crate) blink_on: bool,
    blink_epoch: u64,
    blink_task: Option<Task<()>>,
    /// Text the input method is composing, drawn at the cursor.
    pub(crate) marked: Option<String>,
    /// Which characters (not ASCII) the terminal font draws one cell wide.
    pub(crate) fits: HashMap<char, bool>,
    pub(crate) fits_font: Option<(gpui::SharedString, Pixels)>,
    /// Frames painted and snapshots taken (for measurements and tests).
    pub(crate) frames: u64,
    snapshots: u64,
    /// What each frame showed, while a measurement records it.
    frame_log: Option<Vec<FrameRecord>>,
    /// The first wakeup since the last snapshot, and the wakeup and copy
    /// times of the snapshot shown (measurements).
    woke: Option<Instant>,
    shown_times: (Option<Instant>, Option<Instant>),
    _events: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<SessionViewEvent> for SessionView {}

impl Focusable for SessionView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SessionView {
    /// A view that starts a session with `options` once it is laid out
    /// (the session starts with the size of the view; the spawn runs off
    /// the UI thread).
    pub fn new(options: SessionOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self::empty(window, cx);
        view.pending = Some(options);
        view
    }

    /// A view of a local shell with the start rules of the reference
    /// (shell, arguments, working directory, environment), without product
    /// customizations.
    pub fn local(
        request: &terminal::StartRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(terminal::plan_session(request, &[]).options, window, cx)
    }

    /// A view of a session that is already running.
    pub fn with_session(
        session: Session,
        events: flume::Receiver<SessionEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::empty(window, cx);
        view.attach(session, events, cx);
        view
    }

    fn empty(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |this, window, cx| {
                if let Some(s) = &this.session {
                    s.set_focused(true);
                }
                this.restart_blink(window, cx);
                cx.notify();
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                if let Some(s) = &this.session {
                    s.set_focused(false);
                }
                this.blink_task = None;
                this.blink_on = true;
                cx.notify();
            }),
        ];
        Self {
            session: None,
            pending: None,
            pending_input: String::new(),
            error: None,
            exit: None,
            title: None,
            working_directory: None,
            command_running: false,
            focus,
            options: TerminalOptions::default(),
            snapshot: Snapshot::default(),
            spare: Snapshot::default(),
            copy_in_frame: false,
            changed: false,
            copying: None,
            copy_epoch: 0,
            unshown: false,
            frame_requested: false,
            refresh_task: None,
            last_copy: None,
            busy_since: None,
            window: window.window_handle(),
            resolver: Resolver::default(),
            palette: None,
            geometry: None,
            size: None,
            gesture: Gesture::None,
            last_motion: None,
            wheel_remainder: 0.0,
            blink_on: true,
            blink_epoch: 0,
            blink_task: None,
            marked: None,
            fits: HashMap::new(),
            fits_font: None,
            frames: 0,
            snapshots: 0,
            frame_log: None,
            woke: None,
            shown_times: (None, None),
            _events: None,
            _subscriptions: subscriptions,
        }
    }

    fn attach(
        &mut self,
        session: Session,
        events: flume::Receiver<SessionEvent>,
        cx: &mut Context<Self>,
    ) {
        if let Some(size) = self.size {
            session.resize(size);
        }
        if !self.pending_input.is_empty() {
            session.write_text(&std::mem::take(&mut self.pending_input));
        }
        self.palette = None;
        self.session = Some(session);
        self.copy_in_frame = true;
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                if this.update(cx, |v, cx| v.on_event(event, cx)).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Starts the pending session with the size of the view.
    pub(crate) fn start_pending(&mut self, size: TerminalSize, cx: &mut Context<Self>) {
        let Some(mut options) = self.pending.take() else {
            return;
        };
        options.size = size;
        self.size = Some(size);
        let spawn = cx
            .background_executor()
            .spawn(async move { Session::spawn(options) });
        self._events = Some(cx.spawn(async move |this, cx| {
            let result = spawn.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok((session, events)) => v.attach(session, events, cx),
                Err(e) => {
                    v.error = Some(e.to_string());
                    cx.notify();
                }
            });
        }));
    }

    fn on_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Wakeup => self.screen_changed(cx),
            SessionEvent::Title(title) => {
                self.title = title.clone();
                cx.emit(SessionViewEvent::TitleChanged(title));
            }
            SessionEvent::Bell => cx.emit(SessionViewEvent::Bell),
            SessionEvent::ClipboardStore(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            SessionEvent::WorkingDirectory(dir) => {
                self.working_directory = Some(dir.clone());
                cx.emit(SessionViewEvent::WorkingDirectoryChanged(dir));
            }
            SessionEvent::Mark(mark) => {
                let running = match mark.kind {
                    MarkKind::CommandExecuted => true,
                    MarkKind::CommandFinished { .. } | MarkKind::PromptStart => false,
                    MarkKind::CommandStart => self.command_running,
                };
                if running != self.command_running {
                    self.command_running = running;
                    cx.emit(SessionViewEvent::CommandRunningChanged(running));
                }
            }
            SessionEvent::Integration(_) => {}
            SessionEvent::Exited(status) => {
                self.exit = Some(status);
                self.show_now(cx);
                cx.emit(SessionViewEvent::Exited(status));
            }
        }
    }

    /// The session reported a change of its screen.
    fn screen_changed(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.frame_log.is_some() && self.woke.is_none() {
            self.woke = Some(now);
        }
        // A change while the last copy is still on its way to the screen,
        // or soon after it: output keeps coming. (The UI thread may see the
        // change late, after a long frame: "soon" is generous.)
        let on_the_way = self.copying.is_some() || self.unshown || self.refresh_task.is_some();
        let soon = self
            .last_copy
            .is_some_and(|copy| now.duration_since(copy) < FLOOD_FRAME);
        if on_the_way || soon {
            self.busy_since.get_or_insert(now);
        } else {
            self.busy_since = None;
        }
        self.changed = true;
        self.pump(cx);
    }

    /// Whether output has kept coming for [`FLOOD_AFTER`].
    fn flooding(&self) -> bool {
        self.busy_since
            .is_some_and(|since| since.elapsed() >= FLOOD_AFTER)
    }

    /// Starts the next copy of the screen when one is due: after a change,
    /// once the last copy was drawn, at most one per [`FRAME`] (per
    /// [`FLOOD_FRAME`] while output floods).
    fn pump(&mut self, cx: &mut Context<Self>) {
        if !self.changed
            || self.session.is_none()
            || self.copying.is_some()
            || self.unshown
            || self.refresh_task.is_some()
        {
            return;
        }
        let interval = if self.flooding() { FLOOD_FRAME } else { FRAME };
        let wait = self
            .last_copy
            .map(|copy| interval.saturating_sub(copy.elapsed()))
            .unwrap_or_default();
        if !wait.is_zero() {
            self.refresh_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |v, cx| {
                    v.refresh_task = None;
                    v.pump(cx);
                });
            }));
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let screen = session.screen();
        let mut buffer = std::mem::take(&mut self.spare);
        let epoch = self.copy_epoch;
        let woke = self.woke.take();
        self.changed = false;
        self.last_copy = Some(Instant::now());
        // The lock may be held by the reader while it parses a batch of
        // output: the UI thread does not wait for it.
        let copy = cx.background_executor().spawn(async move {
            screen.snapshot_into(&mut buffer);
            buffer
        });
        self.copying = Some(cx.spawn(async move |this, cx| {
            let snapshot = copy.await;
            let _ = this.update(cx, |v, cx| v.copied(snapshot, epoch, woke, cx));
        }));
    }

    /// A background copy is done: drawn at once (the owner draws the window
    /// on [`SessionViewEvent::Redraw`]), or while output floods at the start
    /// of the next frame.
    fn copied(
        &mut self,
        snapshot: Snapshot,
        epoch: u64,
        woke: Option<Instant>,
        cx: &mut Context<Self>,
    ) {
        self.copying = None;
        if epoch != self.copy_epoch {
            // A copy in a frame since is newer.
            self.spare = snapshot;
            self.pump(cx);
            return;
        }
        self.spare = std::mem::replace(&mut self.snapshot, snapshot);
        self.snapshots += 1;
        self.shown_times = (woke, Some(Instant::now()));
        self.unshown = true;
        if self.flooding() {
            self.repaint_next_frame(cx);
        } else {
            // An echo waiting for the next refresh would be a frame late:
            // the display refresh that follows a key has usually passed by
            // the time the echo is back from the PTY.
            cx.notify();
            cx.emit(SessionViewEvent::Redraw);
        }
    }

    /// Repaints the view at the start of the next display frame (while
    /// output floods). Notifying now would mark the window for drawing
    /// until then, and GPUI draws a marked window before it dispatches a
    /// key: a key typed in the editor meanwhile would wait for a frame of
    /// the whole window first.
    fn repaint_next_frame(&mut self, cx: &mut Context<Self>) {
        if self.frame_requested {
            return;
        }
        let this = cx.entity().downgrade();
        let requested = cx.update_window(self.window, |_, window, _| {
            window.on_next_frame(move |_, cx| {
                let _ = this.update(cx, |v, cx| {
                    v.frame_requested = false;
                    // Not drawn by a frame for another reason meanwhile.
                    if v.unshown {
                        cx.notify();
                    }
                });
            });
        });
        match requested {
            Ok(()) => self.frame_requested = true,
            Err(_) => cx.notify(),
        }
    }

    /// The user changed what the screen shows (scrolling, selecting): copy
    /// it in the next frame.
    fn show_now(&mut self, cx: &mut Context<Self>) {
        self.busy_since = None;
        self.copy_in_frame = true;
        cx.notify();
    }

    /// Sizes the session to the grid, gives it the theme's colours and
    /// copies the screen when this frame must show it; the copy drawn
    /// now is shown, so the next one may start.
    pub(crate) fn prepare(
        &mut self,
        size: TerminalSize,
        colors: &TerminalColors,
        cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() {
            self.start_pending(size, cx);
        }
        let Some(session) = &self.session else {
            return;
        };
        if self.size != Some(size) {
            self.size = Some(size);
            session.resize(size);
            self.copy_in_frame = true;
        }
        let palette = colors.palette();
        if self.palette.as_ref() != Some(&palette) {
            session.set_palette(palette.clone());
            self.palette = Some(palette);
        }
        if self.copy_in_frame {
            self.copy_in_frame = false;
            // A background copy still running is older.
            self.copy_epoch += 1;
            let start = Instant::now();
            session.snapshot_into(&mut self.snapshot);
            self.changed = false;
            self.last_copy = Some(start);
            self.shown_times = (self.woke.take(), Some(Instant::now()));
            self.snapshots += 1;
        }
        self.unshown = false;
        self.pump(cx);
    }

    // ---- state ----------------------------------------------------------

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_directory.as_ref()
    }

    /// The shell reported that a command runs (its integration's marks):
    /// the reference shows the program's title in the tab only then
    /// (`TerminalTitleUpdating.isCommandRunning`).
    pub fn command_running(&self) -> bool {
        self.command_running && self.exit.is_none()
    }

    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit
    }

    /// Asks the shell, on a background thread, whether a command runs in it
    /// (it has a child process): what a tab checks before it closes
    /// (`TerminalTabCloseListenerImpl.shouldConfirmClosing`). `false` for a
    /// session that did not start or has ended.
    pub fn has_running_commands(&self, cx: &App) -> Task<bool> {
        match (&self.session, self.exit) {
            (Some(session), None) => {
                let probe = session.command_probe();
                cx.background_spawn(async move { probe.has_running_commands() })
            }
            _ => Task::ready(false),
        }
    }

    /// Why the session did not start.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn options(&self) -> &TerminalOptions {
        &self.options
    }

    pub fn set_options(&mut self, options: TerminalOptions, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn geometry(&self) -> Option<Geometry> {
        self.geometry
    }

    /// Frames painted and snapshots taken so far.
    pub fn counters(&self) -> (u64, u64) {
        (self.frames, self.snapshots)
    }

    /// Since when output has kept coming (for measurements and tests): a
    /// pause in it, a key or scrolling ends that, and the next output
    /// starts it anew. A flood once [`FLOOD_AFTER`] has passed since.
    pub fn busy_since(&self) -> Option<Instant> {
        self.busy_since
    }

    /// The view's state as JSON: size, visible lines, cursor, modes,
    /// selection, scrolling, title, working directory and exit.
    pub fn state(&self) -> Value {
        let s = &self.snapshot;
        json!({
            "started": self.session.is_some(),
            "error": self.error,
            "columns": s.columns,
            "rows": s.rows,
            "lines": (0..s.rows).map(|l| s.line_text(l)).collect::<Vec<_>>(),
            "cursor": s.cursor.map(|c| json!({
                "line": c.point.line,
                "column": c.point.column,
                "shape": format!("{:?}", c.shape),
                "blinking": c.blinking,
            })),
            "alternate_screen": s.modes.alt_screen,
            "mouse_reporting": s.modes.mouse != MouseMode::None,
            "bracketed_paste": s.modes.bracketed_paste,
            "selection": self.session.as_ref().and_then(|s| s.selection_text()),
            "display_offset": s.display_offset,
            "history": s.history_size,
            "title": self.title,
            "working_directory": self.working_directory,
            "command_running": self.command_running(),
            "exit_code": self.exit.and_then(|e| e.code),
            "exited": self.exit.is_some(),
        })
    }

    // ---- input ----------------------------------------------------------

    /// Typed or composed text, or text an automation types (kept until the
    /// session starts).
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        match &self.session {
            Some(s) => {
                s.write_text(text);
                self.after_input(cx);
            }
            None if self.pending.is_some() || self._events.is_some() => {
                self.pending_input.push_str(text);
            }
            None => {}
        }
    }

    /// Input went to the session: its echo is copied without the flood's
    /// wait, and the cursor stays visible for a blink interval. The echo's
    /// copy repaints the view.
    fn after_input(&mut self, cx: &mut Context<Self>) {
        self.blink_epoch += 1;
        if !self.blink_on {
            self.blink_on = true;
            cx.notify();
        }
        self.busy_since = None;
        if self.refresh_task.take().is_some() {
            self.pump(cx);
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.send_keystroke(&e.keystroke, cx) {
            cx.stop_propagation();
        }
    }

    /// Sends a key to the session as the view's key handler does (the key
    /// routing of the Terminal tool window hands it the keys of IDE
    /// shortcuts); `false` when the session takes no such key.
    pub fn send_keystroke(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let Some((key, mods)) = keys::session_key(keystroke, self.options.option_as_meta) else {
            return false;
        };
        let sent = session.send_key(key, mods);
        if sent {
            self.after_input(cx);
        }
        sent
    }

    // ---- commands -------------------------------------------------------

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        match self.session.as_ref().and_then(|s| s.selection_text()) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            // Without a selection the key goes to the shell (Ctrl+C).
            None => cx.propagate(),
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.paste_text(&text, cx);
        }
    }

    /// Pastes text (bracketed when the program asked for it).
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(s) = &self.session
            && !text.is_empty()
        {
            s.paste(text);
            self.after_input(cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            s.select_all();
            self.show_now(cx);
        }
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            s.clear();
            self.show_now(cx);
        }
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            s.scroll(scroll);
            self.show_now(cx);
        }
    }

    fn line_up(&mut self, _: &LineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll(Scroll::Lines(1), cx);
    }

    fn line_down(&mut self, _: &LineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll(Scroll::Lines(-1), cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll(Scroll::PageUp, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll(Scroll::PageDown, cx);
    }

    // ---- mouse ----------------------------------------------------------

    /// The program gets this mouse event (it asked for reports; Shift
    /// selects text instead, as in the reference and xterm).
    fn reports_mouse(&self, mods: &GpuiModifiers) -> bool {
        self.options.mouse_reporting && self.snapshot.modes.mouse != MouseMode::None && !mods.shift
    }

    fn mods(m: &GpuiModifiers) -> Modifiers {
        Modifiers {
            shift: m.shift,
            alt: m.alt,
            control: m.control,
        }
    }

    pub(crate) fn mouse_down(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let (Some(geometry), Some(session)) = (self.geometry, self.session.as_ref()) else {
            return;
        };
        if let Some(bar) = geometry.scrollbar
            && e.button == GpuiButton::Left
            && bar.track.contains(&e.position)
        {
            let grab = if bar.thumb.contains(&e.position) {
                e.position.y - bar.thumb.top()
            } else {
                bar.thumb.size.height / 2.0
            };
            self.gesture = Gesture::Scrollbar(grab);
            self.drag_scrollbar(e.position.y, cx);
            return;
        }
        let (point, side) = geometry.cell_at(e.position);
        let button = match e.button {
            GpuiButton::Left => Some(MouseButton::Left),
            GpuiButton::Middle => Some(MouseButton::Middle),
            // The right button opens the context menu; it is not reported.
            _ => None,
        };
        if let Some(button) = button
            && self.reports_mouse(&e.modifiers)
            && session.send_mouse(MouseAction::Press(button), point, Self::mods(&e.modifiers))
        {
            session.clear_selection();
            self.gesture = Gesture::Reporting(button);
            self.last_motion = Some(point);
            return;
        }
        match e.button {
            GpuiButton::Left => {
                // The session's selection: the snapshot may not show a
                // click's empty one yet.
                let has_selection = session.has_selection();
                if e.modifiers.shift && !e.modifiers.alt && has_selection && e.click_count == 1 {
                    session.update_selection(point, side);
                } else {
                    let kind = match e.click_count {
                        0 | 1 if e.modifiers.alt => SelectionKind::Block,
                        0 | 1 => SelectionKind::Simple,
                        2 => SelectionKind::Word,
                        _ => SelectionKind::Line,
                    };
                    session.start_selection(kind, point, side);
                }
                self.gesture = Gesture::Selecting;
                self.show_now(cx);
            }
            GpuiButton::Middle if self.options.paste_on_middle_click => {
                // The primary selection on Linux, else the clipboard.
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                let item = cx.read_from_primary().or_else(|| cx.read_from_clipboard());
                #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                let item = cx.read_from_clipboard();
                if let Some(text) = item.and_then(|c| c.text()) {
                    self.paste_text(&text, cx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (Some(geometry), Some(session)) = (self.geometry, self.session.as_ref()) else {
            return;
        };
        match self.gesture {
            Gesture::Scrollbar(_) => self.drag_scrollbar(e.position.y, cx),
            Gesture::Selecting => {
                // Dragging above or below the grid scrolls the history.
                if e.position.y < geometry.origin.y {
                    session.scroll(Scroll::Lines(1));
                } else if e.position.y
                    > geometry.origin.y + geometry.cell.height * geometry.rows as f32
                {
                    session.scroll(Scroll::Lines(-1));
                }
                let (point, side) = geometry.cell_at(e.position);
                session.update_selection(point, side);
                self.show_now(cx);
            }
            Gesture::Reporting(button) => {
                let (point, _) = geometry.cell_at(e.position);
                if self.last_motion != Some(point) {
                    self.last_motion = Some(point);
                    session.send_mouse(
                        MouseAction::Move(Some(button)),
                        point,
                        Self::mods(&e.modifiers),
                    );
                }
            }
            Gesture::None => {
                // Motion without a button (mode 1003) over the grid.
                if self.snapshot.modes.mouse == MouseMode::Motion
                    && self.reports_mouse(&e.modifiers)
                    && e.pressed_button.is_none()
                    && geometry.bounds.contains(&e.position)
                {
                    let (point, _) = geometry.cell_at(e.position);
                    if self.last_motion != Some(point) {
                        self.last_motion = Some(point);
                        session.send_mouse(
                            MouseAction::Move(None),
                            point,
                            Self::mods(&e.modifiers),
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn mouse_up(&mut self, e: &MouseUpEvent, cx: &mut Context<Self>) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        let (Some(geometry), Some(session)) = (self.geometry, self.session.as_ref()) else {
            return;
        };
        match gesture {
            Gesture::Reporting(button) => {
                let (point, _) = geometry.cell_at(e.position);
                session.send_mouse(
                    MouseAction::Release(button),
                    point,
                    Self::mods(&e.modifiers),
                );
            }
            Gesture::Selecting => {
                if let Some(text) = session.selection_text() {
                    if self.options.copy_on_selection {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    }
                    // The selection is the primary selection on Linux.
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    cx.write_to_primary(ClipboardItem::new_string(text));
                    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                    let _ = text;
                }
            }
            Gesture::Scrollbar(_) | Gesture::None => {}
        }
        self.last_motion = None;
    }

    /// The wheel over the view: reported to the program, arrow keys on the
    /// alternate screen, or the history scrolled.
    pub fn scroll_wheel(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let (Some(geometry), Some(session)) = (self.geometry, self.session.as_ref()) else {
            return;
        };
        let line_height = geometry.cell.height;
        self.wheel_remainder += f32::from(e.delta.pixel_delta(line_height).y);
        let lines = (self.wheel_remainder / f32::from(line_height)).trunc() as i32;
        if lines == 0 {
            return;
        }
        self.wheel_remainder -= lines as f32 * f32::from(line_height);
        let modes = self.snapshot.modes;
        if self.reports_mouse(&e.modifiers) {
            let (point, _) = geometry.cell_at(e.position);
            let button = if lines > 0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                session.send_mouse(MouseAction::Press(button), point, Self::mods(&e.modifiers));
            }
            return;
        }
        if modes.alt_screen {
            if self.options.alternate_scroll && !e.modifiers.shift {
                session.send_alternate_scroll(lines);
            }
            return;
        }
        session.scroll(Scroll::Lines(lines));
        self.show_now(cx);
    }

    fn drag_scrollbar(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let (Gesture::Scrollbar(grab), Some(geometry), Some(session)) =
            (self.gesture, self.geometry, self.session.as_ref())
        else {
            return;
        };
        let Some(bar) = geometry.scrollbar else {
            return;
        };
        let room = bar.track.size.height - bar.thumb.size.height;
        if room <= px(0.0) {
            return;
        }
        let top = (y - grab - bar.track.top()).clamp(px(0.0), room);
        // The thumb at the top shows the oldest line: the largest offset.
        let ratio = f32::from(top) / f32::from(room);
        let target = ((1.0 - ratio) * bar.history as f32).round() as i32;
        let current = self.snapshot.display_offset as i32;
        if target != current {
            session.scroll(Scroll::Lines(target - current));
            self.show_now(cx);
        }
    }

    // ---- cursor blinking ------------------------------------------------

    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink_on = true;
        self.blink_epoch += 1;
        if !self.options.blink {
            self.blink_task = None;
            return;
        }
        self.blink_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut seen = None;
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    // Input shows the cursor for a full interval.
                    if seen == Some(this.blink_epoch) {
                        this.blink_on = !this.blink_on;
                    } else {
                        this.blink_on = true;
                    }
                    seen = Some(this.blink_epoch);
                    let blinks = this.snapshot.cursor.is_some_and(|c| c.blinking);
                    if blinks {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    pub(crate) fn note_frame(&mut self) {
        self.frames += 1;
        if let Some(log) = &mut self.frame_log {
            log.push(FrameRecord {
                at: Instant::now(),
                snapshots: self.snapshots,
                cursor: self.snapshot.cursor.map(|c| c.point),
                display_offset: self.snapshot.display_offset,
                tail: (0..self.snapshot.rows)
                    .rev()
                    .map(|l| self.snapshot.line_text(l))
                    .filter(|t| !t.trim().is_empty())
                    .take(FrameRecord::TAIL)
                    .collect(),
                woke: self.shown_times.0,
                copied: self.shown_times.1,
            });
        }
    }

    /// Starts (or stops) recording what each frame shows.
    pub fn record_frames(&mut self, on: bool) {
        self.frame_log = on.then(Vec::new);
    }

    /// The frames recorded so far (the log keeps recording).
    pub fn take_frame_log(&mut self) -> Vec<FrameRecord> {
        self.frame_log
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub(crate) fn set_geometry(&mut self, geometry: Geometry) {
        self.geometry = Some(geometry);
    }

    pub(crate) fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }
}

impl Render for SessionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = ui::theme::color(theme.raw().ui[UiRole::TerminalDefaultBackground]);
        let base = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .size_full()
            .bg(background);
        match &self.error {
            Some(error) => base
                .p(px(12.0))
                .text_color(theme.color(UiRole::Danger))
                .child(format!(
                    "The terminal session could not be started: {error}"
                )),
            None => base.child(TerminalElement::new(cx.entity())),
        }
    }
}
