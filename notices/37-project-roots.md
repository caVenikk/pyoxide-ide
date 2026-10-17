- crates/platform/project/src/roots.rs, store.rs and idea.rs (roots),
  crates/platform/workbench/src/project_panel/roots.rs, crates/python/
  pyoxide-ide/src/lib.rs (tree hooks),
  docs/adr/0013-source-roots-and-packages.md
  Source roots, packages and modules in the project tree follow the
  IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): platform/lang-impl/src/com/
  intellij/ide/projectView/impl/nodes/PsiDirectoryNode.java and
  ProjectViewDirectoryHelper.java (bold content roots, the hint with the
  kind of root and the location), platform/lang-impl/src/com/intellij/psi/
  impl/file/SourceRootIconProvider.java, platform/lang-impl/src/com/
  intellij/openapi/roots/ui/configuration/ModuleSourceRootEditHandler.java
  and JavaModuleSourceRootEditHandler.java, platform/lang-impl/src/com/
  intellij/ide/projectView/actions/MarkRootActionBase.java,
  MarkSourceRootAction.java, MarkTestSourceRootAction.java,
  MarkExcludeRootAction.java and UnmarkRootAction.java, python/resources/
  META-INF/pycharm-core.xml (PyCharm.MarkRootGroup), python/src/com/
  jetbrains/python/PyDirectoryIconProvider.kt and PythonModuleTypeBase.java,
  python/python-psi-impl/src/com/jetbrains/python/psi/PyUtil.java (a
  package has __init__.py), python/ide/impl/resources/
  intellij.pycharm.community.ide.impl.xml (the pyproject.toml model is on
  in PyCharm), the strings of platform/ide-core/resources/messages/
  ProjectBundle.properties and platform/lang-api/resources/messages/
  LangBundle.properties ("{0} Root", "Sources", "Test Sources", "Unmark as
  {0}", "Cancel Exclusion"). Only behaviour and strings are used;
  reimplemented in Rust, no code is copied.
