- crates/platform/workbench/src/main_toolbar.rs, src/chrome.rs (toolbar
  widgets), src/run/widget.rs (the configuration selector), the status bar
  in src/workbench.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the size distribution of
  platform/editor-ui-api/src/com/intellij/openapi/actionSystem/toolbarLayout/
  CompressingLayoutStrategy.kt (distributeSize), the left / centre / right
  placement of platform/platform-api/src/com/intellij/ui/components/panels/
  HorizontalLayout.kt with the gap of platform/platform-impl/src/com/
  intellij/openapi/wm/impl/headertoolbar/MainToolbar.kt, the text cut and
  minimum width of platform/platform-impl/src/com/intellij/openapi/wm/impl/
  DefaultCutStrategy.java, ide/ui/laf/darcula/ui/AbstractToolbarComboUI.kt
  and platform/util/ui/src/com/intellij/util/ui/UIUtil.java
  (computeTextComponentMinimumSize), the run configuration's cut and minimum
  of platform/execution-impl/src/com/intellij/execution/ui/
  RedesignedRunWidget.kt (trimRunConfigurationName, splitCharCount,
  getMinimumSize), and the status bar's layout of platform/platform-impl/
  src/com/intellij/openapi/wm/impl/status/IdeStatusBarImpl.kt. Changes:
  reimplemented in Rust; the integer water filling is solved as a
  continuous level; texts are measured once per frame from shaped glyph
  positions.
