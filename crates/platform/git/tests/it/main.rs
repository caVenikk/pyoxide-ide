//! The integration tests of `git` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod branches;
mod commit;
mod perf;
mod remote;
mod repo;
