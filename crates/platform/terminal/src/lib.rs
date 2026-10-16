//! Terminal sessions of Pyoxide IDE (ADR 0014), without a UI toolkit.
//!
//! - [`Session`]: a shell on a pseudo-terminal (openpty on macOS and Linux,
//!   ConPTY on Windows 10 1809 and later) with the emulator state it writes
//!   into. A thread per session reads the output, pre-scans it for OSC 7
//!   and OSC 133 ([`prescan`]) and feeds it to the emulator; input, resize,
//!   hang-up and kill go through the same thread; [`SessionEvent`]s arrive
//!   on a channel.
//! - [`Snapshot`]: what the view paints, copied under the terminal's lock at
//!   most once per frame (from any thread through [`Screen`]): the visible
//!   cells, the cursor, the modes, the selection and the search matches.
//! - [`input`]: xterm encoding of keys, pastes, mouse and focus events.
//! - [`start`]: the start rules of the reference (shell, `--login` / `-i`,
//!   working directory, environment) and [`plan_session`] with the
//!   [`ShellCustomizer`] product hook; [`shells`]: the shells found on the
//!   machine.
//! - [`integration`]: Pyoxide's shell integration scripts (zsh, bash, fish,
//!   PowerShell) that source what customizers ask for before the first
//!   prompt and report the working directory, the prompt and the commands.
//!
//! The emulator is `alacritty_terminal` (Apache-2.0); its types do not leave
//! this crate. `tty::setup_env` is never called: the environment is passed
//! per session, without the window id variables `tty` adds on Unix
//! ([`start::ALACRITTY_VARS`]).

mod conpty;
mod customizer;
pub mod input;
pub mod integration;
mod io_loop;
pub mod prescan;
mod session;
pub mod shells;
mod snapshot;
pub mod start;

pub use conpty::{MIN_CONPTY_BUILD, unsupported_windows_build};
pub use customizer::{
    Customization, ShellContext, ShellCustomizer, SourceScript, StartPlan, StartRequest, home_dir,
    plan_session,
};
pub use input::{Key, Modifiers, MouseAction, MouseButton};
pub use prescan::{MarkKind, OWN_OSC};
pub use process::ExitStatus;
pub use session::{
    DEFAULT_SCROLLBACK, MatchCount, Palette, Screen, Scroll, SearchDirection, SearchError,
    SearchQuery, SelectionKind, Session, SessionEvent, SessionOptions, ShellMark, Side, SpawnError,
    TerminalSize,
};
pub use shells::{DetectedShell, ShellEnvironment, detect_shells};
pub use snapshot::{
    Cell, CellExtra, CellFlags, Color, Cursor, CursorShape, Hyperlink, MAX_VISIBLE_MATCHES, Modes,
    MouseMode, Point, Rgb, Snapshot, Span,
};
pub use start::{ShellCommand, ShellKind};
