//! The integration tests of `editor` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod basics;
mod console;
mod context_menu;
mod decorations;
mod decorations_perf;
mod editing;
mod marks;
mod problems;
mod soft_wraps;
