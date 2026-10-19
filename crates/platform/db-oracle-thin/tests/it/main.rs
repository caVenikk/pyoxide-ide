//! The integration tests of `db-oracle-thin` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod common;

mod bench;
mod conformance;
mod introspection;
mod oracle;
