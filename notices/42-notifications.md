- crates/platform/ui/src/notification.rs, crates/platform/workbench/src/
  notifications.rs and notifications/tool_window.rs
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3:
  notification balloons of platform/platform-impl/src/com/intellij/
  notification/impl/NotificationsManagerImpl.java (layout of icon, title,
  content and action links, display times of BALLOON and STICKY_BALLOON,
  the content's wrap width and line limit, the expand and collapse
  chevrons of CenteredLayoutWithActions and HoverAdapter), ui/
  NotificationsUtil.java (the width style of long content),
  ui/BalloonLayoutConfiguration.java (sizes and spaces), ui/
  ActionCenterBalloonLayout.kt (three shown, the "{0} more {1}" strip,
  position), ui/NotificationBalloonActionProvider.java (close button),
  ui/NotificationBalloonRoundShadowBorderProvider.kt and ui/BalloonImpl.java
  (smart fade-out: from the first input, paused while hovered or
  inactive); the Notifications tool window of notification/impl/
  NotificationsToolWindow.kt and notification/impl/ui/NotificationsPanel.kt
  (empty text, the Timeline group with "Clear all", the layout of a
  notification and its time), with the strings of platform/platform-api/
  resources/messages/IdeBundle.properties and the chevron mapping of
  platform/icons/src/PlatformIconMappings.json. Changes: reimplemented in
  Rust on GPUI; plain-text content without HTML, wrapped by GPUI's line
  wrapper; no settings button, Suggestions group, search, unread state or
  menus in the tool window.
