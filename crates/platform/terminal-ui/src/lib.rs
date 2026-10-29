//! Terminal view of Pyoxide IDE (ADR 0014), on GPUI.
//!
//! - [`SessionView`]: one session of the `terminal` crate in a focusable
//!   view: keys, typed and composed text, pastes, mouse selection and
//!   reporting, wheel and scrollbar, resizes, and the Terminal commands.
//!   It copies the screen on a background thread, at most once per
//!   [`FRAME`], and asks its owner to draw the copy at once
//!   ([`SessionViewEvent::Redraw`]); while output floods, once per
//!   [`FLOOD_FRAME`], repainted at the start of the next display frame.
//! - [`TerminalElement`]: the grid element that paints the view's snapshot
//!   ([`plan`] works out a frame without the toolkit: cell backgrounds,
//!   text runs at their columns, the cursor).
//! - [`colors`]: the theme's terminal colours with the reference's rules
//!   for inverse and dim text and its minimum contrast.
//!
//! - [`tool_window`]: the Terminal tool window, its tabs and their
//!   commands.

pub mod colors;
pub mod commands;
mod element;
mod ime;
pub mod keys;
pub mod plan;
pub mod tool_window;
mod view;

pub use element::{
    ALTERNATE_LEFT_INSET, BOTTOM_INSET, CellMetrics, LEFT_INSET, TOP_INSET, TerminalElement,
    cell_metrics, grid_layout,
};
pub use terminal::StartRequest;
pub use tool_window::{TerminalToolWindow, TerminalToolWindowEvent};
pub use view::{
    BLINK_INTERVAL, FLOOD_AFTER, FLOOD_FRAME, FRAME, FrameRecord, Geometry, KEY_CONTEXT,
    ScrollbarGeometry, SessionView, SessionViewEvent, TerminalOptions,
};

/// Registers the Terminal commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::TAB_COMMANDS);
    // Nothing started in a terminal outlives the application (#280): the
    // shells and their programs are ended before the process exits.
    cx.on_app_quit(|_| {
        terminal::end_all_sessions(terminal::QUIT_GRACE);
        async {}
    })
    .detach();
}
