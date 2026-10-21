- crates/platform/workbench/src/workbench/project_widget.rs,
  crates/platform/workbench/src/recent_projects.rs,
  crates/platform/workbench/src/chrome.rs (the project widget's avatar),
  crates/platform/ui/src/avatar.rs, crates/platform/ui/src/menu.rs (rows
  with an avatar, popups that scroll beyond a number of rows), the theme
  role `selection-light`
  The project widget's popup, the recent projects and the project avatars
  follow IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ tag pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): platform/platform-impl/src/
  com/intellij/openapi/wm/impl/headertoolbar/ProjectToolbarWidgetAction.kt
  (the popup's groups, their order and titles, the row layout and its
  gaps, the speed search over name and folder, the light selection),
  platform/platform-impl/src/com/intellij/ide/RecentProjectsManagerBase.kt,
  RecentProjectListActionProvider.kt and ReopenProjectAction.kt (the list's
  order, the projects open in windows, the folders kept when they are
  gone, the `~` paths), RecentProjectIconHelper.kt and
  ProjectWindowCustomizerService.kt (the palette, a new project's colour
  after the last one given), platform/util/ui/src/com/intellij/util/ui/
  AvatarUtils.kt (the initials, the gradient, the corners and the font of
  an avatar), platform/platform-api/src/com/intellij/openapi/ui/popup/
  util/PopupUtil.java and platform/util/ui/src/com/intellij/util/ui/
  JBUI.java (the selection's insets, the popup's metrics), the colours
  `RecentProject.ColorN.Avatar` of platform/platform-resources/src/themes/
  expUI/expUI_dark.theme.json and expUI_light.theme.json and
  `Plugins.lightSelectionBackground` of themes/islands/
  ManyIslandsDark.theme.json and ManyIslandsLight.theme.json, the action
  texts of platform/platform-resources-en/src/messages/ActionsBundle.properties.
  Only behaviour, numbers, colours and texts are used; reimplemented in
  Rust with GPUI.
