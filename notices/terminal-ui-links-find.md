- crates/platform/terminal-ui/src/blocks.rs, crates/platform/terminal-ui/src/element.rs
  (block_decor), crates/platform/terminal-ui/src/find.rs,
  crates/platform/terminal-ui/src/link_ui.rs, crates/platform/terminal-ui/src/links.rs,
  keymaps/*/terminal.toml (the find keys), keymaps/actions/edit.toml
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  insets of the command blocks and the separator between them (blockTopInset,
  blockBottomInset, the separator one pixel above the block and its right
  offset, no room above the first block), the marker beside a failed block
  (width, offset, corner radius, vertical inset) of
  plugins/terminal/frontend/src/com/intellij/terminal/frontend/view/impl/
  TerminalBlocksDecorator.kt, the renderers of the separator and the marker
  and the constants of platform/execution-impl/src/com/intellij/terminal/
  block/ui/TerminalUi.kt; the find bar's width (searchComponentWidth), its
  controls (no whole words, no replace) and the search colours of
  TerminalSearchController.kt; the hint over a clicked path that opens with
  Cmd (Ctrl): its delay (editor.invisible.hyperlink.popup.delay.ms), its
  text "Open" with the shortcut, and when it hides, of
  platform/lang-impl/src/com/intellij/execution/impl/
  InvisibleHyperlinkHintManager.kt.
  Changes: reimplemented in Rust on GPUI; the blocks come from the marks of
  the shell integration (OSC 133) and are kept by line number, so they are
  dropped when the scrollback is full or the width changes; the links of the
  screen are scanned per snapshot, the product's filters run on the
  background executor.
