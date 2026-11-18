- crates/platform/code-intel-ui/src/list_popup.rs (File Structure settings
  and navigation), crates/platform/settings/src/file_structure.rs
  Adapted from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  platform/structure-view-impl/src/com/intellij/ide/util/FileStructurePopup.java.
  The gear menu's sorting and speed-search choices, persistence of toggle
  state, movable and resizable frame, and Enter/F4 navigation are implemented
  with the application's Rust settings store and GPUI components.
