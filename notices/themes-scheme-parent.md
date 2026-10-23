- crates/platform/themes/schemes/DefaultColorSchemesManager.xml, PythonDarcula.xml,
  PythonDefault.xml
  Copied from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2). Files:
  platform/platform-resources/src/DefaultColorSchemesManager.xml (the `Default`
  and `Darcula` colour schemes), python/pluginResources/colorSchemes/
  PythonDarcula.xml, PythonDefault.xml. Changes: none to the files; the
  importer (crates/platform/themes/src/import/bundled.rs) reads them as the
  `parent_scheme` that theme files name but do not contain.
