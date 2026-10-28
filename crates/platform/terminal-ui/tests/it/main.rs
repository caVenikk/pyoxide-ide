//! The integration tests of `terminal-ui` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod blocks;
mod find;
mod links;
mod perf;
mod view;
