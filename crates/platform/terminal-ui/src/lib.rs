//! Terminal view of Pyoxide IDE (ADR 0014), on GPUI.
//!
//! - [`SessionView`]: one session of the `terminal` crate in a focusable
//!   view: keys, typed and composed text, pastes, mouse selection and
//!   reporting, wheel and scrollbar, resizes, and the Terminal commands.
//! - [`TerminalElement`]: the grid element that paints the view's snapshot
//!   ([`plan`] works out a frame without the toolkit: cell backgrounds,
//!   text runs at their columns, the cursor).
//! - [`colors`]: the theme's terminal colours with the reference's rules
//!   for inverse and dim text and its minimum contrast.
//!
//! The Terminal tool window (tabs, key routing) builds on these.

pub mod colors;
pub mod commands;
mod element;
mod ime;
pub mod keys;
pub mod plan;
mod view;

pub use element::{
    ALTERNATE_LEFT_INSET, BOTTOM_INSET, CellMetrics, LEFT_INSET, TOP_INSET, TerminalElement,
    cell_metrics, grid_layout,
};
pub use terminal::StartRequest;
pub use view::{
    BLINK_INTERVAL, FRAME, FrameRecord, Geometry, KEY_CONTEXT, ScrollbarGeometry, SessionView,
    SessionViewEvent, TerminalOptions,
};

/// Registers the Terminal commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
}
