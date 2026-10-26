- crates/platform/progress/src/lib.rs, crates/platform/ui/src/progress_bar.rs,
  crates/platform/workbench/src/background_tasks.rs,
  crates/platform/code-intel-ui/src/server_tasks.rs, the progress bar roles
  of crates/platform/themes/themes/*.toml, the `ShowProcessWindow` row of
  crates/platform/workbench/keymaps/actions/window.toml, and the icons
  crates/platform/ui/assets/icons/progress/stop.svg, stopHovered.svg,
  pause.svg, pauseHovered.svg, resume.svg, resumeHovered.svg,
  status/success.svg (with their `_dark.svg` variants)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2): the status
  bar widget of platform/platform-impl/src/com/intellij/openapi/wm/impl/
  status/InfoAndProgressPanel.kt (the text, the 104 px bar and its spaces,
  the cancel button, the `+N` counter of `CounterLabel`, "Hide processes"
  while the popup is open, the tooltip, the pause button and
  `updateCancelButton`, Auto Show) and ProgressComponent.kt (the text
  falling back to the title, "Stopping - …", the bar's value
  `fraction * 99 + 1`); the Processes popup of ProcessPopup.java (size,
  place, title, Hide button, Escape, staying open on a click outside,
  moving and resizing), AutoShowProcessPopupAction.java and the
  `ide.windowSystem.autoShowProcessPopup` registry key, the paused text of
  platform/platform-impl/src/com/intellij/openapi/progress/impl/
  ProgressSuspender.java,
  TasksFinishedDecorator.kt and platform/platform-api/src/com/intellij/
  openapi/ui/panel/ProgressPanelBuilder.java (a row: title, bar, button,
  comment and details, the separator between rows and their spaces);
  ProgressUIUtil's show delay; the indeterminate and determinate drawing
  of platform/platform-impl/src/com/intellij/ide/ui/laf/darcula/ui/
  DarculaProgressBarUI.java and the `ProgressBar.*` colours of the Islands
  themes; the strings of platform/platform-api/resources/messages/
  IdeBundle.properties and IndexingBundle.properties; the `BackgroundTasks`
  group of platform/platform-resources/src/idea/PlatformActions.xml. Icons:
  platform/icons/src/expui/progress/stop.svg, stopHovered.svg, pause.svg,
  pauseHovered.svg, resume.svg, resumeHovered.svg and
  platform/icons/src/expui/status/success.svg, unchanged copies. Changes:
  reimplemented in Rust on GPUI with a registry of tasks that has no UI
  toolkit; the bar is drawn by a replaceable painter; no secondary
  progresses; the popup keeps its place while the window is open, not
  across sessions.
