//! The integration tests of `workbench` in one test program, a module per
//! area: each test program links the crate and all its dependencies
//! (issue #246), so a new area is a new module here, not a new file in
//! `tests/`.

mod mouse;
mod support;

mod appearance;
mod branches;
mod breakpoints;
mod code_intel;
mod code_intel_docs;
mod code_intel_nav;
mod commit;
mod debug_sessions;
mod diff;
mod drop;
mod editor_font;
mod exclusions;
mod external_changes;
mod file_ops;
mod file_type_icons;
mod find;
mod find_in_files;
mod first_run;
mod keymap;
mod keymaps;
mod main_menu;
mod modules;
mod navigation;
mod notifications;
mod pointer;
mod popup_frame;
mod project;
mod project_widget;
mod remote;
mod run;
mod run_configurations;
mod scroll;
mod search_everywhere;
mod settings;
mod shell;
mod soft_wraps;
mod source_roots;
mod terminal;
mod tool_windows;
mod toolbar_layout;
mod tree_rows;
mod version_control;
mod welcome;
mod window_layout;
