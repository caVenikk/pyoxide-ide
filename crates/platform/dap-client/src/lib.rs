//! The Debug Adapter Protocol client of Pyoxide IDE (ADR 0015).
//!
//! A [`Client`] talks to one debug adapter over a [`Transport`] (an adapter
//! process's standard streams, or a socket to an adapter that listens):
//! typed requests that return [`Request`] futures (dropping one cancels the
//! request), events and reverse requests (`runInTerminal`,
//! `startDebugging`) passed to a [`Handler`], and
//! [`Client::shutdown`], which ends the adapter with `terminate` and
//! `disconnect` and kills what is left after a timeout.
//!
//! The design follows the LSP client (ADR 0010): one reader and one writer
//! thread, answers through callbacks and oneshot slots, no async runtime,
//! `Content-Length` frames from `stream-framing`. The protocol's messages
//! are serde structs private to the crate; the public API is in [`types`].
//! The adapter's own launch arguments stay JSON values.
//!
//! The `dap-fake-adapter` binary runs a script of steps instead of a real
//! adapter, for the tests (the steps are described in `src/script.rs`);
//! with the feature `fake`, [`fake::adapter`] runs the same scripts in the
//! test's process.

mod client;
mod protocol;
#[cfg(feature = "fake")]
mod script;
mod transport;
pub mod types;

#[cfg(feature = "fake")]
pub mod fake;

pub use client::{Client, Handler, Request, Responder};
pub use transport::{Incoming, Inlet, MemoryPeer, Outlet, Transport};
pub use types::*;
