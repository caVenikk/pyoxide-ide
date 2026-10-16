- crates/platform/terminal-ui (colors.rs, plan.rs, element.rs, view.rs,
  keys.rs, commands.rs), crates/platform/themes/src/contrast.rs
  (ensure_contrast), the terminal roles of crates/platform/themes
  (roles.rs, import/mapping.rs), keymaps/macos.toml and keymaps/default.toml
  (the Terminal context)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the minimum contrast
  adjustment (ensureContrastRatio, reduceLuminance, increaseLuminance,
  shouldIgnoreContrastAdjustment), the colours of inverse and dim text
  and the required ratio of plugins/terminal/src/org/jetbrains/plugins/
  terminal/block/ui/TerminalUiUtils.kt and TerminalContrastRatio.kt; the
  insets of block/ui/TerminalUi.kt; the cursor shapes, the hollow block of
  an unfocused view and the cursor colour of frontend/src/com/intellij/
  terminal/frontend/view/impl/TerminalCursorPainter.kt; the cell size and
  the alternate screen without scrollbars of TerminalEditorFactory.kt and
  TerminalUiUtils.kt (getCharSize); mouse reporting with Shift selecting
  of TerminalMouseEventsHandlerImpl.kt; the text and background palette of
  platform/execution-impl/src/com/intellij/terminal/BlockTerminalColors.kt
  and TerminalColorPalette.kt (BLOCK_TERMINAL_* keys); the actions and
  their keys of frontend/resources/intellij.terminal.frontend.xml and
  resources/META-INF/plugin.xml. Changes: reimplemented in Rust on GPUI
  as a grid element instead of an editor in a character-grid mode.
