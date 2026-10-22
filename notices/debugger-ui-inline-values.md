- crates/platform/debugger-ui (values after the lines, src/inline.rs; values
  under the mouse and their popups, src/value_hint.rs; the popup mode of the
  variables tree), crates/platform/editor (hover look of after-line texts, the
  background of a range), crates/python/py-debug (src/declarations.rs, where a
  variable is declared; src/hover.rs, the expression under the mouse),
  crates/platform/themes (DEBUGGER_INLINED_VALUES*,
  EVALUATED_EXPRESSION_ATTRIBUTES, EVALUATED_EXPRESSION_EXECUTION_LINE_ATTRIBUTES
  and CTRL_CLICKABLE), crates/platform/workbench/keymaps (Quick Evaluate
  Expression)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the inline debugger of
  platform/xdebugger-impl/ui/src/com/intellij/xdebugger/impl/inline/
  (InlineDebugRenderer.java with LinePainter's changed values, item by item
  for lists and dicts; InlineDebugRendererBase.java: the layout, the hovered
  colour and the drop-down triangle; XDebuggerTreeInlayPopup.java,
  DebuggerInlayListener.java, XDebuggerInlayUtil.java); the value hints of
  platform/xdebugger-impl/ui/src/com/intellij/platform/debugger/impl/ui/
  evaluate/quick/common/ValueLookupManager.java and
  xdebugger/impl/evaluate/quick/ (XValueHint.java, common/AbstractValueHint.java,
  XDebuggerTooltipPopup.java, XDebuggerTreePopup.java, XDebuggerPopupPanel.java:
  the 700 ms delay, "Evaluating…" after 200 ms, the tooltip's insets and its
  Documentation and Problems action, the tree popup's size and its toolbar
  links); the Python expression under the mouse of
  python/src/com/jetbrains/python/debugger/PyDebugSupportUtils.java and
  PyDebuggerEvaluator.kt; the icon platform/icons/src/expui/status/errorOutline
  (light and dark, copied unchanged); the colours of
  platform/platform-resources/src/DefaultColorSchemesManager.xml and
  themes/islands/IslandSchemeDark.xml, expUI/expUI_lightScheme.xml; strings of
  XDebuggerBundle.properties; the key of QuickEvaluateExpression of
  platform/platform-resources/src/keymaps/$default.xml. Changes: reimplemented
  in Rust with GPUI; declarations and expressions are found with
  tree-sitter-python instead of the reference's PSI; Add as Inline Watch adds
  a watch to the Variables view (there are no watches after the lines).
