- crates/platform/themes/themes/*.toml (metrics), docs/theme-mapping.md
  Layout metrics of the New UI with islands taken from the IntelliJ Platform
  Community (Apache-2.0), https://github.com/JetBrains/intellij-community @
  f5a0bddb9cee9135c700de9a67534e11cdf61c07:
  platform/platform-resources/src/themes/islands/ManyIslandsDark.theme.json
  (Island.arc, Tree.rowHeight, TabbedPane.tabHeight, MainToolbar.Icon.insets),
  platform/util/ui/src/com/intellij/util/ui/JBUI.java (header, toolbar and
  stripe defaults), platform/platform-resources/src/themes/darcula.theme.json
  (tree indents), platform/platform-impl/src/com/intellij/openapi/application/
  impl/islands/IslandsTabPainter.kt (tab highlight height and radius).
  Only numbers are used; no code is copied.
