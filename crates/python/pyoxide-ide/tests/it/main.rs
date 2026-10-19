//! The integration tests of `pyoxide-ide` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod debug_sessions;
mod interpreter;
mod live_services;
mod run_configurations;
mod terminal_venv;
