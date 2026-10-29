//! SQL language services of Pyoxide IDE (ADR 0022) over the metadata model
//! of `db-model` and the syntax tree of `sql-syntax`.
//!
//! - [`Scope`], [`SqlContext`] and [`ScopeProvider`]: what names in a
//!   document mean (a data source's model and a search path) and its
//!   dialect, supplied by `db-ui`.
//! - Resolution: every name of a statement resolves to a [`Resolution`]:
//!   `Resolved`, `Ambiguous`, `Unresolved`, or `Unknown` whenever the answer
//!   cannot be known (no scope, a schema outside the model or loaded to
//!   names only, a hole, procedural code). Only `Unresolved` is reported.
//!   Query scopes know table aliases, CTEs, subquery columns with `*`
//!   expanded, objects created earlier in the script, Oracle's `DUAL`,
//!   PostgreSQL's system columns and the dialect's pseudo-columns.
//! - [`Inspection`]s of the first set with their [`Fix`]es, levels from an
//!   [`InspectionProfile`], `-- noinspection` comments.
//! - [`Analyzer`]: the analysis of a document, statement by statement, with
//!   the results of unchanged statements kept.
//! - [`SqlCodeIntel`]: the services as a `CodeIntel` provider for the
//!   language id `"sql"`: diagnostics in the background and code actions.
//!
//! No UI toolkit, no network, no async runtime.

mod analysis;
mod builtins;
mod complete;
mod context;
mod cursor;
mod docs;
mod hover;
mod inspect;
mod intel;
mod locals;
mod names;
mod profile;
mod reference;
mod resolve;
mod signature;

pub use analysis::{Analyzer, DocumentAnalysis, analyze};
pub use context::{CompletionOptions, FixedScope, KeywordCase, Scope, ScopeProvider, SqlContext};
pub use inspect::{Fix, FixEdit, Problem};
pub use intel::{SERVER_ID, SqlCodeIntel};
pub use profile::{Inspection, InspectionLevel, InspectionProfile};
pub use reference::{RefKind, Reference, Resolution, Suggestion, Target};
