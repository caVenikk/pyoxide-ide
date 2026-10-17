- crates/platform/workbench/keymaps/actions.toml,
  crates/platform/workbench/src/keymap/ (actions.rs, active.rs, derived.rs,
  reference.rs), crates/platform/workbench/src/settings/keymap.rs,
  crates/platform/settings/src/keymap.rs, crates/platform/ui/src/menu.rs
  (bold and indented rows)
  The action table, the keymap model and the Keymap page follow IntelliJ
  Platform Community (Apache-2.0), https://github.com/JetBrains/
  intellij-community @ tag pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2).
  Action ids, texts and the groups of the actions tree: the action files
  platform/platform-resources/src/idea/PlatformActions.xml, LangActions.xml
  and ExecutionActions.xml, platform/vcs-impl/resources/META-INF/
  VcsActions.xml, python/resources/META-INF/pycharm-core.xml; strings of
  platform/platform-resources-en/src/messages/ActionsBundle.properties,
  platform/platform-api/resources/messages/IdeBundle.properties
  (toolwindow.stripe.*) and KeyMapBundle.properties; the order of the
  tree's groups of platform/platform-impl/src/com/intellij/openapi/keymap/
  impl/ui/ (ActionsTreeUtil.java, ActionTreeGroupUtil.kt). The keymap file
  format, parent chain and the replacing of an action's shortcuts of
  KeymapImpl.kt, KeymapManagerImpl.kt, DefaultKeymap.kt and
  MacOSDefaultKeymap.kt in platform/platform-impl/src/com/intellij/openapi/
  keymap/impl/, and the keymaps platform/platform-resources/src/keymaps/
  ($default.xml, Mac OS X 10.5+.xml) for the parents' names and the
  `use-shortcut-of` aliases. Page metrics (the combo box, the link row, the
  toolbar, the tree's rows and indent, the key caps' height, radius,
  widths and gaps) and behaviour: KeymapPanel.java, ActionsTree.java,
  ShortcutTextUi.kt and ShortcutFilteringPanel.java in the same ui/ folder,
  platform/platform-impl/src/com/intellij/application/options/schemes/
  SchemesCombo.java (bold predefined schemes, indented custom ones), and the
  Keymap popup of platform/platform-impl/src/com/intellij/ide/actions/
  QuickChangeKeymapAction.java. Changes: reimplemented in Rust
  with GPUI; only the actions Pyoxide has are listed, with the Pyoxide
  command each runs and, where one action is several commands, a key
  context; the tree is read-only; Import Keymap… takes the place of the
  link to the keymap plugins.
