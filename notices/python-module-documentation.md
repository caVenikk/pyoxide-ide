- crates/python/pyoxide-ide/src/module_doc.rs (the table of standard
  library pages and the docs.python.org link of a module)
  Closely adapted from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the modules and page names of
  py3LibraryModulesToWebpageName and the URL rules of
  python/src/com/jetbrains/python/codeInsight/stdlib/
  PyStdlibDocumentationLinkProvider.java (getExternalDocumentationRoot,
  getStdlibUrlFor, getModuleName); pyVersion and getOnlyUrlFor of
  python/python-psi-impl/src/com/jetbrains/python/documentation/
  PythonDocumentationProvider.java; the header "Module {0}" / "Package {0}"
  of PyDocumentationBuilder.java (addModulePath) and python/python-psi-impl/
  resources/messages/PyPsiBundle.properties. Changes: the table is a Rust
  array of the same entries; the module comes from the type server's hover
  and its declaration instead of the reference's PSI.
