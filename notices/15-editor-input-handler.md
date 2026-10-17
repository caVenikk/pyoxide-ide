- crates/platform/editor/src/input.rs
  The structure of the platform input handler (UTF-16 conversions, marked
  text, replace-and-mark) follows GPUI's text input example (Apache-2.0),
  https://github.com/zed-industries/zed @ 69e2130 (the commit of the
  crates.io gpui 0.2.2 release), file crates/gpui/examples/input.rs.
  Changes: works on the rope buffer with multiple selections, undo groups and
  the editor's own layout for character bounds.
