//! The Language Server Protocol client of Pyoxide IDE (ADR 0010).
//!
//! A product describes its servers with [`ServerSpec`]s (command line,
//! root, languages, the features each serves, settings, a
//! [`ServerCustomizer`]); [`LspCodeIntel`] runs them and implements
//! [`code_intel::CodeIntel`] over all of them, routing each feature to the
//! servers that serve it.
//!
//! The design (one `process` child per server, a reader thread that parses
//! frames and answers pending requests through oneshot channels, requests
//! cancelled by dropping their futures, no async runtime) is in ADR 0010.
//! Protocol types (`lsp-types`) never appear in the public API.
//!
//! Modules:
//!
//! - `framing`: `Content-Length` frames over chunks of a byte stream;
//! - `rpc`: JSON-RPC requests, notifications, server requests and
//!   cancellation over a process's standard streams;
//! - `capabilities`: what the client advertises and reads at `initialize`;
//! - `encoding`: position encodings and conversion between the model's
//!   points and LSP positions;
//! - `server`: one server's lifecycle and the requests it sends the client,
//!   with `server::sync`: its documents and their diagnostics;
//! - `documents`, `diagnostics`: the open documents with their recent
//!   history, and every server's diagnostics of them;
//! - `features`, `convert`: the language features and the conversion of
//!   answers to the model;
//! - `glob`: the file watchers servers register;
//! - `intel`: [`LspCodeIntel`].

mod capabilities;
mod convert;
mod customizer;
mod debounce;
mod diagnostics;
mod documents;
mod encoding;
mod features;
mod framing;
mod glob;
mod intel;
mod rpc;
mod server;
mod spec;
mod texts;
mod uri;

pub use customizer::{NoCustomization, ServerCustomizer, lookup_section};
pub use intel::LspCodeIntel;
pub use server::Timeouts;
pub use spec::ServerSpec;
