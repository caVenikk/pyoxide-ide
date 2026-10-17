- crates/platform/workbench/src/search_everywhere/ (pages, footer, scope
  mnemonics), crates/python/pyoxide-ide/src/main.rs (source roots)
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/261.27258.51:
  the page sizes of platform/lang-impl/src/com/intellij/ide/actions/
  searcheverywhere/SearchEverywhereUI.java, the mnemonics of
  ScopeChooserAction.java, the footer's path of footer/ExtendedInfoImpl.kt,
  and the source roots that python/ide/impl/src/com/intellij/pycharm/
  community/ide/impl/PythonSourceRootConfigurator.java marks when a folder
  opens. Reimplemented in Rust; `setup.py` is read as text, not parsed as
  Python.
