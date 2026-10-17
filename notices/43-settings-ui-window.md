- crates/platform/settings-ui (window.rs, tree.rs, search.rs, history.rs,
  leave.rs, window_state.rs, assets/search-ignore.txt),
  crates/platform/workbench/src/settings.rs and settings/system.rs
  Closely adapted from the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  Settings window's layout, metrics, order of buttons and behaviour of
  platform/platform-impl/src/com/intellij/openapi/options/newEditor/
  SettingsEditor.java, SettingsTreeView.java (row renderer, modified and
  error colours, project mark, the group header of the scrolled tree),
  SettingsFilter.kt and SettingsSearch.java (100 ms delay, the choice of
  the page to select, Up and Down from the field, Escape),
  ConfigurableEditorBanner.java, SettingsNonModalDialog.kt (one window,
  Unsaved Changes on project close, quit and project switch) and
  SpotlightPainter.kt; the search rules of
  platform/platform-impl/src/com/intellij/ide/ui/search/
  SearchableOptionsRegistrarImpl.kt (name and content hits, word prefixes,
  paths) and SearchUtil.kt (which labels light up), the spotlight outline
  of platform/platform-impl/src/com/intellij/openapi/options/ex/
  GlassPanel.java, the history of platform/platform-api/src/com/intellij/
  ui/navigation/History.java and of SearchTextField.java (five searches),
  the gear menu of platform/platform-impl/src/com/intellij/ide/actions/
  SettingsEntryPointAction.java with platform/platform-resources/src/idea/
  PlatformActions.xml; strings of ApplicationBundle.properties,
  UIBundle.properties, IdeBundle.properties and CommonBundle.properties;
  the stop words platform/platform-resources-en/src/search/ignore.txt
  (an unchanged copy). Changes: reimplemented in Rust with GPUI; pages are
  found by their titles and declared search terms instead of an index
  built from their components, word prefixes stand in for stemming, the
  spotlight is an outline around the labels found without the overlay
  that dims the rest of the page, and Pin Window is drawn but always on.
