//! The metadata model of a data source (ADR 0021): what the Database tool
//! window shows and what SQL resolution and completion read (ADR 0022),
//! available at once from the disk cache, before any connection.
//!
//! - [`Model`]: an immutable value, schemas shared between versions through
//!   `Arc<SchemaModel>`. Built from an introspection [`Snapshot`], merged with
//!   a [`Delta`] (idempotently), trimmed by a list of live objects. Every
//!   change returns a new model; a cancelled change returns
//!   [`Cancelled`] and leaves nothing half done.
//! - Nodes live in an arena per schema: the schema, its objects (partitions
//!   nested under their table, indexes and triggers under theirs), the
//!   columns, keys and indexes of relations, the arguments of routines.
//!   [`NodeRef`] reads one; [`GroupRef`] is a group of children as the tree
//!   shows them, empty groups included and marked so.
//! - [`NodeId`]: from the node's kind and qualified name, never from a
//!   server oid, so tree state keyed by id survives refreshes and restarts.
//! - Lookups by schema, name and kind are hash lookups that do not allocate,
//!   with exact or case-insensitive names ([`NameMatch`]); the SQL layer
//!   folds identifiers by its dialect's rules before asking. Name search
//!   for speed search uses `name-match`.
//! - [`ModelHandle`]: the current `Arc<Model>` of a data source, swapped
//!   whole by a refresh.
//! - [`ModelCache`]: the model on disk, one versioned JSON file per data
//!   source in the user cache folder, written atomically; [`SourceStore`]:
//!   source texts beside it, read lazily.
//!
//! No UI toolkit, no async runtime, no SQL parsing.
//!
//! [`Snapshot`]: db_core::schema::Snapshot
//! [`Delta`]: db_core::schema::Delta

mod cache;
mod id;
mod kind;
mod merge;
mod model;
mod name;
mod schema;
mod source;

pub use cache::{CACHE_FORMAT, CACHE_VERSION, CacheMiss, ModelCache};
pub use id::{NodeId, ParseNodeIdError};
pub use kind::{Group, NodeKind};
pub use model::{Found, Model, ModelHandle};
pub use name::NameMatch;
pub use schema::{Cancelled, GroupRef, NodeRef, SchemaModel};
pub use source::{SourceKey, SourceStore};
