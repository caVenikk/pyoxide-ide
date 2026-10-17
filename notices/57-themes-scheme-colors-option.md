- crates/platform/themes/src/import/scheme.rs (the `<colors>` option
  `BACKGROUND`)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the handling of the deprecated
  background colour in platform/editor-ui-ex/src/com/intellij/openapi/editor/
  colors/impl/AbstractColorsScheme.java (readScheme, readColors). Changes:
  reimplemented in Rust over the raw option values of one scheme; the
  importer reports the colour under its own name.
