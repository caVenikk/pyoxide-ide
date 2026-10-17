- crates/platform/editor/src/decorations.rs, crates/platform/editor/src/gutter.rs,
  crates/platform/ui/assets/icons/breakpoints/breakpoint.svg
  The editor decorations follow the IntelliJ Platform Community (Apache-2.0,
  license text in LICENSE-APACHE), https://github.com/JetBrains/intellij-community
  @ tag pycharm/2026.2.3: the New UI gutter areas, gaps and icon placement of
  platform/platform-impl/src/com/intellij/openapi/editor/impl/
  EditorGutterComponentImpl.java and EditorGutterLayout.java (icons over line
  numbers, the hover icon's opacity), the change marker sizes of
  platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerDrawUtil.java,
  the thin and wide error stripe marks of .../editor/impl/EditorMarkupModelImpl.kt,
  the highlighter layers of platform/xdebugger-api/src/com/intellij/xdebugger/
  ui/DebuggerColors.java and the inline value placement of platform/
  xdebugger-impl/ui/src/com/intellij/xdebugger/impl/inline/
  InlineDebugRendererBase.java. Only behaviour and numbers are used;
  reimplemented in Rust. The icon is an unchanged copy of platform/icons/src/
  expui/breakpoints/breakpoint.svg.
