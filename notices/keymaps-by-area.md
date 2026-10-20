- crates/platform/workbench/keymaps/macos/*.toml, default/*.toml and
  actions/*.toml
  The former keymaps/macos.toml, default.toml and actions.toml, split into
  one file per area (per menu for the action table) without changing their
  content; the entries that name those three files (for example
  06-keymaps-default.md and keymaps-actions-page.md) describe these folders.
  Source and licence are those of the entries: IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3, the keymaps in platform/platform-resources/src/keymaps/
  ($default.xml, Mac OS X 10.5+.xml) and the action files named in
  keymaps-actions-page.md. Changes: the files were cut by area; the order in
  which the workbench joins them keeps the order of the bindings.
