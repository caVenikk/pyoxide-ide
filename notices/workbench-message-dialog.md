- crates/platform/workbench/src/dialog.rs
  Layout of the message dialogs of IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  platform/platform-impl/src/com/intellij/ui/messages/AlertMessagesManager.kt
  (AlertDialog: width 370 or 440, insets, gaps between the icon, title,
  message, check box and buttons, button sizes, the location a quarter of
  the window's height from its top, the button focused on macOS).
  Changes: reimplemented in Rust on GPUI as a window of its own without a
  title bar (ui::dialog_window); a few distances measured on the
  reference instead of the Swing insets.
