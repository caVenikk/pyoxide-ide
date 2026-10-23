- crates/platform/workbench/src/main_menu.rs,
  crates/platform/workbench/src/workbench/window_actions.rs, the main menu
  rows of crates/platform/workbench/keymaps/actions/file.toml and
  window.toml, the macOS bindings of Minimize and Zoom in
  crates/platform/workbench/keymaps/macos/window.toml
  The main menu follows IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ tag pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the `MainMenu` group and its
  menus in platform/platform-resources/src/idea/PlatformActions.xml,
  LangActions.xml and ExecutionActions.xml, platform/vcs-impl/resources/
  META-INF/VcsActions.xml, plugins/git4idea/backend/resources/
  intellij.vcs.git.backend.xml (`Git.MainMenu`), python/resources/META-INF/
  pycharm-core.xml (`PlatformOpenProjectGroup`), python/pluginResources/
  intellij.python.community.impl.xml (`StepIntoMyCode`) and platform/
  non-modal-welcome-screen/resources/
  intellij.platform.ide.nonModalWelcomeScreen.xml (`OpenWelcome` in Help):
  the order of the menus and their rows, separators and submenus; the
  texts of platform/platform-resources-en/src/messages/
  ActionsBundle.properties and plugins/git4idea/shared/resources/messages/
  GitBundle.properties; the order of Tool Windows (platform/platform-impl/
  src/com/intellij/ide/actions/ToolWindowsGroup.java), Recent Projects
  without the window's project (RecentProjectsGroup.kt, platform/
  platform-impl/src/com/intellij/ide/ReopenProjectAction.kt) and the
  "Current File" submenu of Git (plugins/git4idea/backend/src/actions/
  GitFileActionGroup.kt); the shortcuts of MinimizeCurrentWindow and
  ZoomCurrentWindow in platform/platform-resources/src/keymaps/
  "Mac OS X 10.5+.xml". Only structure, texts and keystrokes are used;
  reimplemented in Rust with GPUI.
