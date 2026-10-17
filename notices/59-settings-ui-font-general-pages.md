- crates/platform/settings-ui/src/pages/font.rs, src/pages/general.rs,
  crates/platform/settings/src/editor.rs, crates/platform/ui/src/theme.rs
  (the chosen editor font), crates/platform/ui/src/form.rs (on_off,
  segmented), crates/platform/workbench/src/editor_settings.rs,
  crates/python/pyoxide-ide/src/settings_pages.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the Font page of
  platform/platform-impl/src/com/intellij/application/options/editor/fonts/
  (AppFontPanel.java, AppFontOptionsPanel.kt, AppEditorFontOptionsPanel.kt,
  FontFamilyCombo.java: the known monospaced families, monospaced ones
  first), the preview text of platform/platform-impl/src/com/intellij/
  application/options/colors/FontPreviewService.kt, the limits of
  platform/editor-ui-ex/src/com/intellij/application/options/
  EditorFontsConstants.java; the Soft Wraps group of platform/lang-impl/
  src/com/intellij/application/options/editor/EditorOptionsPanel.kt; the
  size and colours of platform/platform-api/src/com/intellij/ui/components/
  OnOffButton.java; the Type Engine page of python/python-lsp-core/src/com/
  intellij/python/lsp/core/typeEngine/PyTypeEngineConfigurable.kt; the
  External Tools page of python/python-pytools-ui/src/com/intellij/python/
  pytools/ui/configuration/ (PyExternalToolsConfigurable.kt,
  PyExternalToolsTable.kt, PyExternalToolsHeaderBar.kt,
  PyExternalToolsLabels.kt, PyExternalToolsCellRenderers.kt,
  PyExternalToolsRowModel.kt: the columns, their widths, the lookup chain
  with its marks and colours, the path column); the Python and Tools groups
  of python/ide/impl/resources/intellij.pycharm.community.ide.impl.xml;
  strings of ApplicationBundle.properties, PyBundle.properties,
  PyLspCoreBundle.properties, PyToolsUiBundle.properties and
  RuffBundle.properties. Changes: reimplemented in Rust with GPUI; the font
  is kept in Pyoxide's settings store over the theme's; Pyoxide's type
  servers (ty, basedpyright) take the place of the reference's engines,
  with a description of basedpyright of its own.
