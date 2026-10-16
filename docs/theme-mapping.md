# Theme mapping

Pyoxide has its own theme model (crate `themes`). This document lists how the
keys of PyCharm colour schemes (`.icls`, scheme XML) and UI themes
(`.theme.json`) map to it, how a value is chosen when several keys compete, and
what is approximated or dropped. The key names are format identifiers and are
written as they appear in the files; no colour values of third-party themes are
recorded here. The tables below are checked against the code: a test fails when
a key or a role is missing from this file.

## The theme model

A theme assigns a value to every role of three kinds, plus fonts:

- **UI roles** (`ui.*`, 110 of them): flat colours of the interface, the
  editor chrome, version control, diagnostics and the terminal.
- **Text roles** (`syntax.*`, 53): syntax tokens and editor decorations.
  Each has text attributes: foreground `fg`, background `bg`, `bold`, `italic`,
  `underline` (`single`, `double`, `wavy`, `dotted`, `dashed`, `thick`) with
  `underline-color`, `strikethrough` and a `border` colour. Unset attributes
  inherit from the editor defaults.
- **Metrics** (`metrics.*`, 28): layout numbers in logical pixels (row height,
  gutter width, paddings, radii, ...).
- **Fonts** (`font.*`): UI, editor and terminal families (a list, first match
  wins), sizes, editor line spacing and ligatures.

Themes are TOML files. A file can `extends` another theme and then lists only
what it changes; colours can refer to entries of a `[palette]` table with
`"$name"`. The built-in themes `pyoxide-dark` and `pyoxide-light` define every
role; the importers always start from one of them. The default themes of the
IDE are `islands-dark` and `islands-light` (see "Built-in themes").

```toml
[theme]
id = "my-dark"
name = "My Dark"
extends = "pyoxide-dark"

[palette]
rust = "#c8643c"

[ui]
accent = "$rust"

[syntax]
keyword = { fg = "$rust", bold = true }
```

## Themes in the IDE

The Theme list of Settings > Appearance & Behavior > Appearance, the Theme
popup (gear menu, Quick Switch Scheme) and `settings.switch-theme` offer
the built-in themes and the imported ones, grouped as the reference's:
the Islands themes, the other built-in themes, then "Custom". "Import
Theme…" takes a `.theme.json`, an `.icls` or scheme `.xml`, a theme plugin
`.jar` / `.zip` or a folder, and writes every theme found as
`themes/<id>.toml` in the configuration folder (ADR 0017); an id of a
built-in theme gets `-imported` appended. A colour scheme imported alone is
written with `scheme-only = true` in `[theme]`: it is offered as an editor
colour scheme only. The theme in use is `[appearance] theme` of
`settings.toml`. An editor colour scheme chosen apart from the theme
(`[appearance] editor-scheme`) replaces every text role and the UI roles
that have a colour scheme key among their sources (the editor, version
control markers, diagnostics, the console); choosing another theme goes back
to that theme's own scheme.

## Built-in themes

| Id | Name | Colours |
|---|---|---|
| `islands-dark` | Islands Dark | default dark theme, generated from the IntelliJ Platform Community New UI files |
| `islands-light` | Islands Light | default light theme, generated the same way |
| `pyoxide-dark` | Pyoxide Dark | original colours; base of every import |
| `pyoxide-light` | Pyoxide Light | original colours; base of every import |

The islands themes are produced by this importer from the IntelliJ Platform
Community repository (Apache-2.0, see NOTICE): the Islands UI themes with
their parent themes, the editor schemes they name, the base schemes and the
Python attribute lists. The generated files are committed
(`crates/platform/themes/themes/islands-*.toml`; their header names the
commit and the files) and are regenerated, not edited:

```sh
git clone --filter=blob:none --no-checkout https://github.com/JetBrains/intellij-community.git ic
cd ic && git sparse-checkout set --no-cone \
  /platform/platform-resources/src/themes/ \
  /platform/platform-resources/src/DefaultColorSchemesManager.xml \
  /python/pluginResources/colorSchemes/
git checkout <commit> && cd -
cargo run -p themes --example builtin_islands -- ic <commit>
```

Roles the Community files do not define, the metrics and the fonts come from
the pyoxide themes, so layout changes go into `pyoxide-*.toml` first and the
islands themes are regenerated afterwards. One attribute the reference draws
in code rather than through its schemes is added by the generator: a hint
of the Python language services (`syntax.diagnostic-hint`, empty in the
schemes) is a grey bold dotted line.

## What is read

| Input | How it is used |
|---|---|
| `*.icls` and scheme `*.xml` | `<scheme name version parent_scheme>` with `<colors>`, `<attributes>`, font options and `<font>`, `<console-font>` |
| language attribute lists (`*.xml` whose root is `<list>`) | `<option name>` attribute entries that a language adds to a base scheme; the base is named by the end of the file name (`...Darcula.xml` for `Darcula`, `...Default.xml` for `Default`). They apply below that scheme's own entries, to every scheme that inherits from it, also when the base scheme itself is not among the inputs |
| `*.theme.json`, and `*.json` under `themes/` | `name`, `dark`, `author`, `parentTheme`, `editorScheme`, `colors`, `ui`; other sections are only counted |
| `*.jar`, `*.zip` | only the entries recognised by name below are opened |
| a directory | searched recursively (symbolic links are not followed); archives up to two folders down are opened |

Archives and directories are data sources only. An entry is opened when its
name says it is theme data, decided before the entry is touched:

- `*.theme.json` anywhere, except under `META-INF/`;
- `*.json` under a `themes/` folder (not `metadata/`, not `*.themeMetadata.json`);
- `*.icls` anywhere, and `*.xml` under a `themes/` or `colorSchemes/` folder;
- `DefaultColorSchemesManager.xml` (the base schemes), wherever it is;
- a `.xml` or `.icls` that a theme names in `editorScheme` by path.

Class files, other XML (such as `plugin.xml`), images and everything else are
never read. Entries with `..` in the path are ignored, and an entry larger than
8 MiB is skipped. A file that has the right name but is not a theme or a scheme
is skipped and listed in the report of the run.

`editorScheme` is either a path inside the plugin (`/themes/X.xml`) or the name
of a scheme. A theme without `editorScheme` uses the base scheme of its
brightness (`Darcula` for dark, `Default` for light) when it is among the inputs.

## How a value is chosen

1. **Sources in order.** Each role below lists its sources. The first source
   that yields a colour wins; later ones that also exist are reported as
   *shadowed*.
2. **UI theme keys.** For a key such as `Panel.background` the theme and then
   each parent is searched. In each theme the exact key is tried first, then
   the wildcard `*.background` (only for keys with two segments), then the
   platform variants (`...os.default`). A wildcard of a child therefore beats a
   specific key of its parent. When several sources of one role are present, a
   source reached by an exact key beats one reached by a wildcard.
3. **Values.** A UI theme value is `#RRGGBB`, `#RRGGBBAA`, or a name from
   `colors` (names may point to other names; the `colors` of parents are
   visible, the child wins). Numbers may be written as strings. A scheme colour
   is `RRGGBB`, `RRGGBBAA`, or a shorter hex number (`0` is black); an empty
   value means "no colour" and hides the parent's value.
4. **Scheme attributes.** The scheme is searched before its parent schemes.
   `baseAttributes="KEY"` starts an entry from another entry; own options win.
   An entry that exists but is empty replaces the parent's entry.
5. **Editor roles prefer the scheme.** `ui.editor-background` takes the `TEXT`
   background before `Editor.background`.
6. **Derived values.** A role no source supplies takes the value of its
   fallback role (tables below) when that role was imported or derived itself,
   and is listed as an approximation.
7. **Base theme.** Whatever is still missing comes from `pyoxide-dark` or
   `pyoxide-light` (chosen by `dark`; a scheme alone is dark when its editor
   background is dark, else by the name of its base scheme) and is listed in
   the report as taken from the base theme. A text role that a scheme defines
   replaces the base style completely; it is not merged with it.

The base schemes of the IDE (`Default`, `Darcula`) and the base themes are not
shipped here. When they are among the inputs (pass the folder or archive of an
installed IDE as context to the importer) their values are used; otherwise the
report says which parent was missing.

## UI roles

### General

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.surface` | `Panel.background` | Main window and panel background |
| `ui.surface-alt` | `SidePanel.background`, `Tree.background` | Secondary panels such as the project tree and side panels |
| `ui.surface-raised` | `Popup.background`, `PopupMenu.background`, `Menu.background`, `ToolTip.background` | Popups, menus, dialogs and tooltips |
| `ui.input` | `TextField.background`, `ComboBox.background` | Background of text fields and combo boxes |
| `ui.border` | `Borders.color`, `ToolWindow.borderColor`, `Popup.borderColor`, `Menu.borderColor` | Borders between panels |
| `ui.border-input` | `Component.borderColor`, `TextField.borderColor`, `Button.startBorderColor` | Outline of inputs and buttons |
| `ui.divider` | `Separator.separatorColor`, `Menu.separatorColor`, `Popup.separatorColor`, `MainToolbar.separatorColor` | Thin separators inside a panel |
| `ui.popup-border` | `Popup.borderColor`, `Menu.borderColor` | Border of popups and menus, and the separators in menus |
| `ui.text` | `Label.foreground`, `Panel.foreground` | Primary text |
| `ui.text-muted` | `Label.infoForeground`, `Component.infoForeground` | Secondary text and hints |
| `ui.text-disabled` | `Label.disabledForeground`, `Label.disabledText`, `Component.disabledForeground` | Text of disabled controls |
| `ui.text-link` | `Link.activeForeground`, `Link.foreground` | Links in the user interface |
| `ui.shortcut` | `Shortcut.foreground` | Keyboard shortcuts shown after an action, as in the empty editor area |
| `ui.accent` | `Button.default.startBackground`, `Component.accentColor`, `ProgressBar.progressColor`, `Component.focusColor` | Primary action and focus accent |
| `ui.on-accent` | `Button.default.foreground` | Text and icons on the accent colour |
| `ui.focus` | `Component.focusColor`, `Button.default.focusColor` | Keyboard focus ring |
| `ui.hover` | `ActionButton.hoverBackground`, `List.hoverBackground`, `Tree.hoverBackground`, `Table.hoverBackground` (UI theme or scheme color) | Background of a hovered row or button |
| `ui.pressed` | `ActionButton.pressedBackground` (UI theme or scheme color) | Background of a pressed button |
| `ui.selection` | `List.selectionBackground`, `Tree.selectionBackground`, `Table.selectionBackground` | Background of the selected row in lists and trees |
| `ui.selection-text` | `List.selectionForeground`, `Tree.selectionForeground` | Text on the selected row |
| `ui.selection-inactive` | `List.selectionInactiveBackground`, `Tree.selectionInactiveBackground` | Selected row in a list that has no focus |
| `ui.danger` | `Label.errorForeground`, `Component.errorFocusColor` | Errors and destructive actions |
| `ui.warning` | `Label.warningForeground`, `Component.warningFocusColor` | Warnings |
| `ui.success` | `Label.successForeground` | Success and completed states |
| `ui.info` | `Notification.informativeBorderColor` | Informational accents |
| `ui.danger-bg` | `Notification.errorBackground`, `ValidationTooltip.errorBackground`, `Editor.ToolTip.errorBackground` | Background of error notices |
| `ui.warning-bg` | `Notification.warningBackground`, `ValidationTooltip.warningBackground`, `Editor.ToolTip.warningBackground` | Background of warning notices |
| `ui.success-bg` | `Editor.ToolTip.successBackground` | Background of success notices |
| `ui.info-bg` | `Notification.informativeBackground` | Background of informational notices |

### Chrome

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.title-bar` | `TitlePane.background` | Window title bar |
| `ui.toolbar` | `MainToolbar.background`, `Toolbar.background` | Main toolbar |
| `ui.status-bar` | `StatusBar.background` | Status bar background |
| `ui.status-bar-text` | `StatusBar.foreground`, `StatusBar.Widget.foreground` | Status bar text |
| `ui.tool-window` | `ToolWindow.background` | Tool window content background |
| `ui.tool-window-header` | `ToolWindow.Header.background` | Tool window title strip |
| `ui.tool-window-stripe` | `ToolWindow.Stripe.background` | Strip with the tool window buttons |
| `ui.tab-bar` | `EditorTabs.background`, `DefaultTabs.background` | Strip behind the editor tabs |
| `ui.tab-active` | `EditorTabs.underlinedTabBackground`, `EditorTabs.selectedBackground`, `DefaultTabs.underlinedTabBackground` | Selected editor tab |
| `ui.tab-inactive` | `EditorTabs.unselectedBackground` | Editor tab that is not selected |
| `ui.tab-hover` | `EditorTabs.hoverBackground`, `EditorTabs.hoverColor`, `DefaultTabs.hoverBackground` | Hovered editor tab |
| `ui.tab-active-text` | `EditorTabs.underlinedTabForeground`, `EditorTabs.selectedForeground`, `DefaultTabs.underlinedTabForeground` | Title of the selected tab |
| `ui.tab-inactive-text` | `EditorTabs.foreground`, `TabbedPane.foreground` | Title of other tabs |
| `ui.tab-underline` | `EditorTabs.underlinedBorderColor`, `EditorTabs.underlineColor`, `DefaultTabs.underlineColor`, `TabbedPane.underlineColor` | Indicator of the selected tab (the outline of its rounded background in the New UI with islands) |
| `ui.scrollbar` | `ScrollBar.thumbColor` (UI theme or scheme color), `ScrollBar.Transparent.thumbColor` (UI theme or scheme color), `ScrollBar.Mac.thumbColor` (UI theme or scheme color), `ScrollBar.Mac.Transparent.thumbColor` (UI theme or scheme color) | Scrollbar thumb |
| `ui.scrollbar-hover` | `ScrollBar.hoverThumbColor` (UI theme or scheme color), `ScrollBar.Transparent.hoverThumbColor` (UI theme or scheme color), `ScrollBar.Mac.hoverThumbColor` (UI theme or scheme color), `ScrollBar.Mac.Transparent.hoverThumbColor` (UI theme or scheme color) | Hovered scrollbar thumb |
| `ui.scrollbar-track` | `ScrollBar.trackColor` (UI theme or scheme color), `ScrollBar.Transparent.trackColor` (UI theme or scheme color), `ScrollBar.Mac.trackColor` (UI theme or scheme color), `ScrollBar.Mac.Transparent.trackColor` (UI theme or scheme color) | Scrollbar track |
| `ui.progress` | `ProgressBar.progressColor` | Progress bar fill |
| `ui.progress-track` | `ProgressBar.trackColor` | Progress bar track |
| `ui.notification` | `Notification.background` | Background of notification balloons (the scheme colour `NOTIFICATION_BACKGROUND` is the editor's notification panel, not a balloon) |
| `ui.notification-border` | `Notification.borderColor` | Border of notification balloons |
| `ui.notification-text` | `Notification.foreground` | Title and content of notification balloons |
| `ui.notification-link` | `Notification.linkForeground` | Action links of notification balloons |
| `ui.notification-icon-hover` | `Notification.iconHoverBackground` | Background of the hovered close button of a notification balloon |
| `ui.notification-more` | `Notification.MoreButton.background` | Strip below the balloons that counts the collapsed notifications |
| `ui.notification-more-text` | `Notification.MoreButton.foreground` | Text of that strip |
| `ui.non-project-file` | `FileColor.Yellow` | Row background of files outside the project content, such as excluded folders (the reference's "Non-Project Files" file colour) |
| `ui.run-widget-running` | `RunWidget.runningBackground` | Background of the run widget's Run button while its process runs |
| `ui.run-widget-stop` | `RunWidget.stopBackground` | Background of the run widget's Stop button |
| `ui.drag-and-drop-border` | `DragAndDrop.borderColor` | Border of the row that takes dragged files or folders (the reference's drop target rectangle) |
| `ui.drag-and-drop-row` | `DragAndDrop.rowBackground` | Background of the row that takes dragged files or folders |
| `ui.tree-indent-guide` | `Tree.hash` | Indent guides of trees: the vertical lines below an open folder (the reference's "Show indent guides") |
| `ui.list-button-hover` | `List.Button.hoverBackground` | Background of a hovered button at the end of a popup row, such as Run in the run widget's dropdown |
| `ui.list-button-separator` | `List.Button.separatorColor` | Line before the buttons at the end of a popup row |

### Editor

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.editor-background` | `TEXT` background, `Editor.background` | Editor background |
| `ui.editor-text` | `TEXT` foreground, `Editor.foreground` | Default editor text |
| `ui.gutter` | `EDITOR_GUTTER_BACKGROUND` (scheme color) | Gutter background; the New UI reads only this key (`GUTTER_BACKGROUND` of older schemes is not used), and an empty value means the editor background |
| `ui.line-number` | `LINE_NUMBERS_COLOR` (scheme color) | Line numbers |
| `ui.line-number-active` | `LINE_NUMBER_ON_CARET_ROW_COLOR` (scheme color) | Line number of the caret row |
| `ui.caret` | `CARET_COLOR` (scheme color) | Text caret |
| `ui.current-line` | `CARET_ROW_COLOR` (scheme color) | Background of the caret row |
| `ui.editor-selection` | `SELECTION_BACKGROUND` (scheme color) | Selected text background |
| `ui.search-match` | `TEXT_SEARCH_RESULT_ATTRIBUTES` background, `SearchMatch.startBackground` | Background of search matches |
| `ui.search-match-active` | `SEARCH_RESULT_ATTRIBUTES` background | Background of the current search match |
| `ui.search-match-stripe` | `TEXT_SEARCH_RESULT_ATTRIBUTES` error stripe colour | Marks of search matches on the scrollbar |
| `ui.search-field` | `Editor.SearchField.background` | Background of the find bar at the top of the editor |
| `ui.search-field-border` | `Editor.SearchField.borderColor`, `Editor.Toolbar.borderColor` | Line between the find bar and the text |
| `ui.search-field-error` | `SearchField.errorForeground` | Text of a search field whose search finds nothing |
| `ui.search-option-selected` | `SearchOption.selectedBackground` | Background of a search option that is on (Match Case, Words, Regex) |
| `ui.indent-guide` | `INDENT_GUIDE` (scheme color) | Indent guides |
| `ui.indent-guide-active` | `SELECTED_INDENT_GUIDE` (scheme color) | Indent guide of the caret block |
| `ui.right-margin` | `RIGHT_MARGIN_COLOR` (scheme color) | Right margin line |
| `ui.whitespace` | `WHITESPACES` (scheme color) | Visible whitespace markers |
| `ui.soft-wrap-sign` | `SOFT_WRAP_SIGN_COLOR` (scheme color) | Signs at the soft wraps of a line |
| `ui.completion-popup` | `LOOKUP_COLOR` (scheme color) | Completion popup background |
| `ui.completion-selection` | `CompletionPopup.selectionBackground` | Selected completion item |
| `ui.completion-match` | `CompletionPopup.matchForeground` | Matched characters in completion items |
| `ui.documentation-popup` | `DOCUMENTATION_COLOR` (scheme color) | Documentation popup background |
| `ui.breakpoint` | `BREAKPOINT_ATTRIBUTES` background | Line with a breakpoint |
| `ui.execution-point` | `EXECUTIONPOINT_ATTRIBUTES` background | Line the debugger stopped on |
| `ui.bookmark` | `Bookmark.iconBackground` (UI theme or scheme color) | Bookmark marker |

### Version control

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.gutter-added` | `ADDED_LINES_COLOR` (scheme color) | Gutter marker of added lines |
| `ui.gutter-modified` | `MODIFIED_LINES_COLOR` (scheme color) | Gutter marker of modified lines |
| `ui.gutter-deleted` | `DELETED_LINES_COLOR` (scheme color) | Gutter marker of deleted lines |
| `ui.diff-added` | `DIFF_INSERTED` background | Added lines in the diff viewer |
| `ui.diff-removed` | `DIFF_DELETED` background | Removed lines in the diff viewer |
| `ui.diff-changed` | `DIFF_MODIFIED` background | Changed lines in the diff viewer |
| `ui.diff-conflict` | `DIFF_CONFLICT` background | Conflicting lines in the diff viewer |
| `ui.git-added` | `FILESTATUS_ADDED` (scheme color), `FILESTATUS_COPIED` (scheme color) | File name colour: added |
| `ui.git-modified` | `FILESTATUS_MODIFIED` (scheme color) | File name colour: modified |
| `ui.git-deleted` | `FILESTATUS_DELETED` (scheme color) | File name colour: deleted |
| `ui.git-renamed` | `FILESTATUS_RENAMED` (scheme color) | File name colour: renamed |
| `ui.git-untracked` | `FILESTATUS_UNKNOWN` (scheme color) | File name colour: untracked |
| `ui.git-ignored` | `FILESTATUS_IDEA_FILESTATUS_IGNORED` (scheme color) | File name colour: ignored |
| `ui.git-conflict` | `FILESTATUS_IDEA_FILESTATUS_MERGED_WITH_CONFLICTS` (scheme color), `FILESTATUS_changelistConflict` (scheme color) | File name colour: merge conflict |

### Diagnostics

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.diagnostic-error` | `ERRORS_ATTRIBUTES` effect color, `ERRORS_ATTRIBUTES` error stripe color, `ERRORS_ATTRIBUTES` foreground | Error marker, underline and stripe |
| `ui.diagnostic-warning` | `WARNING_ATTRIBUTES` effect color, `WARNING_ATTRIBUTES` error stripe color, `WARNING_ATTRIBUTES` foreground | Warning marker, underline and stripe |
| `ui.diagnostic-info` | `INFO_ATTRIBUTES` effect color, `INFO_ATTRIBUTES` error stripe color, `INFO_ATTRIBUTES` foreground | Weak warning and information marker |
| `ui.diagnostic-hint` | `INFORMATION_ATTRIBUTES` effect color, `INFORMATION_ATTRIBUTES` error stripe color | Hint marker |
| `ui.diagnostic-error-stripe` | `ERRORS_ATTRIBUTES` error stripe color | Error mark on the error stripe and error icon |
| `ui.diagnostic-warning-stripe` | `WARNING_ATTRIBUTES` error stripe color | Warning mark on the error stripe and warning icon |
| `ui.diagnostic-weak-warning-stripe` | `INFO_ATTRIBUTES` error stripe color | Weak warning mark on the error stripe and weak warning icon |

### Terminal

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.terminal-background` | `CONSOLE_BACKGROUND_KEY` (scheme color) | Terminal and console background |
| `ui.terminal-foreground` | `CONSOLE_NORMAL_OUTPUT` foreground | Terminal default text |
| `ui.terminal-black` | `CONSOLE_BLACK_OUTPUT` foreground | ANSI colour 0 |
| `ui.terminal-red` | `CONSOLE_RED_OUTPUT` foreground | ANSI colour 1 |
| `ui.terminal-green` | `CONSOLE_GREEN_OUTPUT` foreground | ANSI colour 2 |
| `ui.terminal-yellow` | `CONSOLE_YELLOW_OUTPUT` foreground | ANSI colour 3 |
| `ui.terminal-blue` | `CONSOLE_BLUE_OUTPUT` foreground | ANSI colour 4 |
| `ui.terminal-magenta` | `CONSOLE_MAGENTA_OUTPUT` foreground | ANSI colour 5 |
| `ui.terminal-cyan` | `CONSOLE_CYAN_OUTPUT` foreground | ANSI colour 6 |
| `ui.terminal-white` | `CONSOLE_GRAY_OUTPUT` foreground | ANSI colour 7 |
| `ui.terminal-bright-black` | `CONSOLE_DARKGRAY_OUTPUT` foreground | ANSI colour 8 |
| `ui.terminal-bright-red` | `CONSOLE_RED_BRIGHT_OUTPUT` foreground | ANSI colour 9 |
| `ui.terminal-bright-green` | `CONSOLE_GREEN_BRIGHT_OUTPUT` foreground | ANSI colour 10 |
| `ui.terminal-bright-yellow` | `CONSOLE_YELLOW_BRIGHT_OUTPUT` foreground | ANSI colour 11 |
| `ui.terminal-bright-blue` | `CONSOLE_BLUE_BRIGHT_OUTPUT` foreground | ANSI colour 12 |
| `ui.terminal-bright-magenta` | `CONSOLE_MAGENTA_BRIGHT_OUTPUT` foreground | ANSI colour 13 |
| `ui.terminal-bright-cyan` | `CONSOLE_CYAN_BRIGHT_OUTPUT` foreground | ANSI colour 14 |
| `ui.terminal-bright-white` | `CONSOLE_WHITE_OUTPUT` foreground | ANSI colour 15 |

Roles without a source are derived, then taken from the base theme.

### Fallback roles for UI roles

| Role | Takes the value of |
|---|---|
| `ui.surface-alt`, `ui.surface-raised`, `ui.input`, `ui.title-bar`, `ui.toolbar`, `ui.status-bar`, `ui.tool-window`, `ui.tab-bar`, `ui.notification`, `ui.editor-background` | `ui.surface` |
| `ui.tool-window-header`, `ui.tool-window-stripe` | `ui.tool-window` |
| `ui.tab-inactive` | `ui.tab-bar` |
| `ui.tab-active` | `ui.editor-background` |
| `ui.tab-active-text`, `ui.tab-inactive-text` | `ui.text` |
| `ui.status-bar-text`, `ui.selection-text` | `ui.text` |
| `ui.selection-inactive` | `ui.selection` |
| `ui.border-input`, `ui.divider`, `ui.popup-border` | `ui.border` |
| `ui.text-disabled`, `ui.shortcut` | `ui.text-muted` |
| `ui.text-link`, `ui.focus`, `ui.progress`, `ui.tab-underline`, `ui.completion-match` | `ui.accent` |
| `ui.scrollbar-hover` | `ui.scrollbar` |
| `ui.completion-popup`, `ui.documentation-popup` | `ui.surface-raised` |
| `ui.completion-selection` | `ui.selection` |
| `ui.gutter`, `ui.terminal-background` | `ui.editor-background` |
| `ui.line-number-active` | `ui.line-number` |
| `ui.caret`, `ui.terminal-foreground` | `ui.editor-text` |
| `ui.indent-guide-active` | `ui.indent-guide` |
| `ui.search-match-active`, `ui.search-match-stripe` | `ui.search-match` |
| `ui.search-field` | `ui.editor-background` |
| `ui.search-field-border` | `ui.border` |
| `ui.search-field-error` | `ui.danger` |
| `ui.search-option-selected` | `ui.pressed` |
| `ui.non-project-file` | `ui.surface-alt` |
| `ui.run-widget-running` | `ui.success` |
| `ui.run-widget-stop` | `ui.danger` |
| `ui.drag-and-drop-border` | `ui.accent` |
| `ui.drag-and-drop-row` | `ui.selection` |
| `ui.tree-indent-guide` | `ui.divider` |
| `ui.list-button-hover` | `ui.hover` |
| `ui.list-button-separator` | `ui.pressed` |
| `ui.notification-border` | `ui.popup-border` |
| `ui.notification-text` | `ui.text` |
| `ui.notification-link` | `ui.text-link` |
| `ui.notification-icon-hover` | `ui.hover` |
| `ui.notification-more` | `ui.notification` |
| `ui.notification-more-text` | `ui.text-muted` |

## Text roles

Keys are attribute names of the scheme. The first one that exists is used; if
several exist with different styles, the others are reported as merged into
the token.

### Syntax

| Pyoxide role | Scheme attribute keys, in order | Used for |
|---|---|---|
| `syntax.keyword` | `PY.KEYWORD`, `DEFAULT_KEYWORD` | Keywords: def, class, if, return, import, ... |
| `syntax.constant-builtin` | none (derived, see below) | True, False, None and Ellipsis |
| `syntax.string` | `DEFAULT_STRING` | String and bytes literals |
| `syntax.string-escape` | `DEFAULT_VALID_STRING_ESCAPE` | Valid escape sequence in a string |
| `syntax.string-escape-invalid` | `DEFAULT_INVALID_STRING_ESCAPE` | Invalid escape sequence in a string |
| `syntax.number` | `PY.NUMBER`, `DEFAULT_NUMBER` | Numeric literals |
| `syntax.comment` | `PY.LINE_COMMENT`, `DEFAULT_LINE_COMMENT`, `DEFAULT_BLOCK_COMMENT` | Line and block comments |
| `syntax.docstring` | `PY.DOC_COMMENT`, `DEFAULT_DOC_COMMENT` | Docstrings and documentation comments |
| `syntax.docstring-tag` | `PY.DOC_COMMENT_TAG`, `DEFAULT_DOC_COMMENT_TAG` | Tags inside docstrings such as :param or @param |
| `syntax.todo` | `TODO_DEFAULT_ATTRIBUTES`, `TODO_ATTRIBUTES` | TODO and FIXME markers in comments |
| `syntax.function-decl` | `DEFAULT_FUNCTION_DECLARATION` | Name in a function definition |
| `syntax.function-call` | `PY.FUNCTION_CALL`, `DEFAULT_FUNCTION_CALL` | Name of a called function |
| `syntax.method-decl` | none (derived, see below) | Name in a method definition |
| `syntax.method-call` | `PY.METHOD_CALL` | Name of a called method |
| `syntax.class` | `DEFAULT_CLASS_REFERENCE`, `DEFAULT_CLASS_NAME` | Reference to a class or type |
| `syntax.class-decl` | `PY.CLASS_DEFINITION`, `DEFAULT_CLASS_NAME` | Name in a class definition |
| `syntax.parameter` | `DEFAULT_PARAMETER` | Function parameter |
| `syntax.self` | `PY.SELF_PARAMETER` | self and cls |
| `syntax.keyword-argument` | `PY.KEYWORD_ARGUMENT` | Name of a keyword argument in a call |
| `syntax.variable` | `DEFAULT_LOCAL_VARIABLE`, `DEFAULT_IDENTIFIER` | Variables and plain identifiers |
| `syntax.variable-reassigned` | `DEFAULT_REASSIGNED_LOCAL_VARIABLE`, `DEFAULT_REASSIGNED_PARAMETER` | Variable that is assigned more than once |
| `syntax.constant` | `DEFAULT_CONSTANT` | Module level constants (UPPER_CASE) |
| `syntax.attribute` | `DEFAULT_INSTANCE_FIELD`, `DEFAULT_STATIC_FIELD` | Instance and class attributes |
| `syntax.builtin` | `PY.BUILTIN_NAME` | Built-in functions and types: len, print, int |
| `syntax.decorator` | `PY.DECORATOR`, `DEFAULT_METADATA` | Decorators |
| `syntax.annotation` | `PY.ANNOTATION`, `DEFAULT_METADATA` | Type annotations |
| `syntax.module` | none (derived, see below) | Module and package names |
| `syntax.dunder` | `PY.PREDEFINED_DEFINITION`, `PY.PREDEFINED_USAGE`, `DEFAULT_PREDEFINED_SYMBOL` | Special names such as __init__ and __name__ |
| `syntax.fstring-expression` | none (derived, see below) | Expression inside an f-string |
| `syntax.fstring-brace` | `PY.FSTRING_FRAGMENT_BRACES`, `PY.FSTRING_FRAGMENT_COLON` | Braces and format spec colon of an f-string |
| `syntax.operator` | `PY.OPERATION_SIGN`, `DEFAULT_OPERATION_SIGN` | Operators |
| `syntax.punctuation` | `PY.COMMA`, `PY.DOT`, `DEFAULT_COMMA`, `DEFAULT_DOT`, `DEFAULT_SEMICOLON` | Commas, dots, colons and semicolons |
| `syntax.bracket` | `PY.PARENTHS`, `PY.BRACKETS`, `PY.BRACES`, `DEFAULT_PARENTHS`, `DEFAULT_BRACKETS`, `DEFAULT_BRACES` | Parentheses, brackets and braces |
| `syntax.tag` | `HTML_TAG_NAME`, `XML_TAG_NAME`, `DEFAULT_TAG` | Markup tags |
| `syntax.tag-attribute` | `HTML_ATTRIBUTE_NAME`, `XML_ATTRIBUTE_NAME`, `DEFAULT_ATTRIBUTE` | Markup attributes |
| `syntax.property-key` | `JSON.PROPERTY_KEY`, `YAML_SCALAR_KEY`, `PROPERTIES.KEY` | Keys in JSON, YAML, TOML and similar |
| `syntax.link` | `HYPERLINK_ATTRIBUTES` | Hyperlinks in the editor |

### Editor decoration

| Pyoxide role | Scheme attribute keys, in order | Used for |
|---|---|---|
| `syntax.diagnostic-error` | `ERRORS_ATTRIBUTES` | Code with an error |
| `syntax.diagnostic-warning` | `WARNING_ATTRIBUTES` | Code with a warning |
| `syntax.diagnostic-info` | `INFO_ATTRIBUTES` | Code with a weak warning |
| `syntax.diagnostic-hint` | `INFORMATION_ATTRIBUTES` | Code with a hint |
| `syntax.diagnostic-unresolved` | `WRONG_REFERENCES_ATTRIBUTES` | Reference that cannot be resolved |
| `syntax.diagnostic-unused` | `NOT_USED_ELEMENT_ATTRIBUTES` | Unused symbol |
| `syntax.diagnostic-deprecated` | `DEPRECATED_ATTRIBUTES` | Deprecated symbol |
| `syntax.typo` | `TYPO` | Spelling mistake |
| `syntax.matching-bracket` | `MATCHED_BRACE_ATTRIBUTES` | Bracket matching the one at the caret |
| `syntax.unmatched-bracket` | `UNMATCHED_BRACE_ATTRIBUTES` | Bracket without a pair |
| `syntax.identifier-read` | `IDENTIFIER_UNDER_CARET_ATTRIBUTES` | Other reads of the symbol at the caret |
| `syntax.identifier-write` | `WRITE_IDENTIFIER_UNDER_CARET_ATTRIBUTES` | Other writes of the symbol at the caret |
| `syntax.folded-text` | `FOLDED_TEXT_ATTRIBUTES` | Placeholder of a collapsed region |
| `syntax.inlay-hint` | `INLAY_DEFAULT`, `INLINE_PARAMETER_HINT` | Inline type and parameter hints |
| `syntax.console-error` | `CONSOLE_ERROR_OUTPUT` | Error output in the run console |
| `syntax.console-input` | `CONSOLE_USER_INPUT` | User input in the run console |
| `syntax.console-system` | `CONSOLE_SYSTEM_OUTPUT` | The IDE's own lines in the run console (command line, exit code) |

### Fallback roles for text roles

A syntax token that no key defines takes the style of its fallback role.
Editor decorations have no fallback except where listed.

| Role | Takes the value of |
|---|---|
| `syntax.constant-builtin`, `syntax.builtin` | `syntax.keyword` |
| `syntax.string-escape` | `syntax.string` |
| `syntax.docstring`, `syntax.todo` | `syntax.comment` |
| `syntax.docstring-tag` | `syntax.docstring` |
| `syntax.function-decl`, `syntax.method-call` | `syntax.function-call` |
| `syntax.method-decl` | `syntax.function-decl` |
| `syntax.class-decl` | `syntax.class` |
| `syntax.parameter`, `syntax.variable-reassigned`, `syntax.constant`, `syntax.attribute`, `syntax.module`, `syntax.fstring-expression`, `syntax.operator`, `syntax.punctuation`, `syntax.bracket`, `syntax.tag-attribute`, `syntax.property-key` | `syntax.variable` |
| `syntax.self-parameter`, `syntax.keyword-argument` | `syntax.parameter` |
| `syntax.decorator` | `syntax.annotation` |
| `syntax.dunder` | `syntax.builtin` |
| `syntax.fstring-brace` | `syntax.punctuation` |
| `syntax.diagnostic-unresolved` | `syntax.diagnostic-error` |

### Attribute conversion

| Scheme option | Pyoxide |
|---|---|
| `FOREGROUND`, `BACKGROUND` | `fg`, `bg` |
| `FONT_TYPE` bit 1 / bit 2 | `bold` / `italic` |
| `EFFECT_TYPE` 0 (box) | `border` in `EFFECT_COLOR` |
| `EFFECT_TYPE` 1 (line) | `underline = "single"` in `EFFECT_COLOR` |
| `EFFECT_TYPE` 2 (wave) | `underline = "wavy"` in `EFFECT_COLOR` |
| `EFFECT_TYPE` 3 (strikeout) | `strikethrough`, drawn in the text colour |
| `EFFECT_TYPE` 4 (bold line) | `underline = "thick"` in `EFFECT_COLOR` |
| `EFFECT_TYPE` 5 (bold dotted) | `underline = "dotted"` in `EFFECT_COLOR` |
| `EFFECT_TYPE` without `EFFECT_COLOR` | no effect, as in the reference, which paints an effect only with both |
| any other `EFFECT_TYPE`, or `EFFECT_COLOR` alone | ignored, listed as an approximation |
| `ERROR_STRIPE_COLOR` | only used as a source of the diagnostic colours above |

The diagnostic roles exist twice on purpose: `ui.diagnostic-*` is the colour of
markers, stripes and icons, `syntax.diagnostic-*` the text decoration in the
editor.

## Metrics

| Metric | UI theme keys | Meaning |
|---|---|---|
| `metrics.row-height` | `Tree.rowHeight`, `List.rowHeight` | Height of a list, tree or table row |
| `metrics.row-height-compact` | none, taken from the base theme | Row height in compact mode |
| `metrics.tree-indent` | none, taken from the base theme | Indentation of one tree level |
| `metrics.tab-height` | none, taken from the base theme | Height of an editor tab |
| `metrics.tab-selection-height` | none, taken from the base theme | Height of the rounded background of the selected or hovered tab |
| `metrics.toolbar-height` | none, taken from the base theme | Height of the main toolbar |
| `metrics.toolbar-button-size` | none, taken from the base theme | Size of an icon button in the main toolbar |
| `metrics.status-bar-height` | none, taken from the base theme | Height of the status bar |
| `metrics.tool-window-header-height` | none, taken from the base theme | Height of a tool window title strip |
| `metrics.stripe-width` | none, taken from the base theme | Width of a tool window stripe |
| `metrics.stripe-button-size` | none, taken from the base theme | Size of a tool window stripe button |
| `metrics.gutter-width` | none, taken from the base theme | Minimum width of the editor gutter |
| `metrics.gutter-padding` | none, taken from the base theme | Horizontal padding inside the gutter |
| `metrics.indent-guide-width` | none, taken from the base theme | Thickness of indent guides |
| `metrics.caret-width` | none, taken from the base theme | Thickness of the text caret |
| `metrics.panel-padding` | none, taken from the base theme | Padding inside panels and tool windows |
| `metrics.padding-small` | none, taken from the base theme | Small gap between related elements |
| `metrics.padding` | none, taken from the base theme | Standard gap between elements |
| `metrics.padding-large` | none, taken from the base theme | Gap between groups of elements |
| `metrics.island-gap` | none, taken from the base theme | Gap between the island panels of the main window and around them |
| `metrics.corner-radius` | `Component.arc`, `Button.arc` (half of the value: an arc is a diameter) | Radius of buttons, inputs and popups |
| `metrics.corner-radius-small` | none, taken from the base theme | Radius of small elements such as tags |
| `metrics.button-radius` | `MainToolbar.Button.arc` (half of the value) | Radius of the hover and selection backgrounds of toolbar, stripe and tab buttons |
| `metrics.island-radius` | `Island.arc` (half of the value) | Radius of the island panels of the main window |
| `metrics.border-width` | none, taken from the base theme | Thickness of borders |
| `metrics.focus-width` | `Component.focusWidth` | Thickness of the focus ring |
| `metrics.scrollbar-width` | none, taken from the base theme | Width of scrollbars |
| `metrics.icon-size` | none, taken from the base theme | Size of toolbar and tree icons |

Only the metrics above can come from a UI theme; all others keep the values of
the base theme.

## Fonts

| Pyoxide | Scheme option |
|---|---|
| `font.editor-family` | `EDITOR_FONT_NAME` (in front of the base families, so missing fonts still fall back) |
| `font.editor-size` | `EDITOR_FONT_SIZE` |
| `font.editor-line-spacing` | `LINE_SPACING` |
| `font.editor-ligatures` | `EDITOR_LIGATURES` |
| `font.terminal-family`, `font.terminal-size` | name and size inside `<console-font>`, or `CONSOLE_FONT_NAME`, `CONSOLE_FONT_SIZE` |

`font.ui-*` is never imported. `FONT_SCALE` and other options under `<scheme>`
are listed as unmapped.

## Parent themes and schemes

`parentTheme` holds an identifier, not a name. The files do not say which
file an identifier belongs to, so the importer looks a parent up among the
available files by file name, then by `name`, and knows the following
identifiers and their parents (observed on real theme files):

| `parentTheme` id | `name` in the file | File name | Parent | Dark |
|---|---|---|---|---|
| `Darcula` | Darcula | `darcula` | none | true |
| `IntelliJ` | IntelliJ | `intellijlaf` | `Darcula` | false |
| `ExperimentalDark` | Dark | `expUI_dark` | `Darcula` | true |
| `ExperimentalLight` | Light | `expUI_light` | `IntelliJ` | false |
| `ExperimentalLightWithLightHeader` | Light with Light Header | `expUI_light_with_light_header` | `ExperimentalLight` | false |
| `Islands Dark` | Islands Dark | `ManyIslandsDark` | `ExperimentalDark` | true |
| `Islands Light` | Islands Light | `ManyIslandsLight` | `ExperimentalLightWithLightHeader` | false |
| `Islands Darcula` | Islands Darcula | `ManyIslandsDarcula` | `ExperimentalDark` | true |
| `HighContrast` | High Contrast | `HighContrast` | `Darcula` | true |

A known identifier whose file is not available is reported as a note and the
chain continues with its parent; an identifier that is not in the table and not
found is reported as a warning (`unknown-parent-theme`). A `parent_scheme`
that is not among the inputs is a note for `Default` and `Darcula` and a
warning for any other name.

## Highlighter names

The editor resolves tokens from its highlighters to the roles above.

### tree-sitter captures

A capture that is not listed uses the longest listed prefix at a dot
boundary (`@keyword.control.return` is a `keyword`).

| Capture | Role |
|---|---|
| `@keyword` | `syntax.keyword` |
| `@boolean` | `syntax.constant-builtin` |
| `@constant` | `syntax.constant` |
| `@constant.builtin` | `syntax.constant-builtin` |
| `@constant.numeric` | `syntax.number` |
| `@number` | `syntax.number` |
| `@float` | `syntax.number` |
| `@string` | `syntax.string` |
| `@character` | `syntax.string` |
| `@string.escape` | `syntax.string-escape` |
| `@string.documentation` | `syntax.docstring` |
| `@string.special.key` | `syntax.property-key` |
| `@string.special.url` | `syntax.link` |
| `@comment` | `syntax.comment` |
| `@comment.documentation` | `syntax.docstring` |
| `@comment.todo` | `syntax.todo` |
| `@comment.note` | `syntax.todo` |
| `@comment.warning` | `syntax.todo` |
| `@comment.error` | `syntax.todo` |
| `@function` | `syntax.function-decl` |
| `@function.call` | `syntax.function-call` |
| `@function.macro` | `syntax.function-call` |
| `@function.method` | `syntax.method-decl` |
| `@function.method.call` | `syntax.method-call` |
| `@function.builtin` | `syntax.builtin` |
| `@function.decorator` | `syntax.decorator` |
| `@constructor` | `syntax.class` |
| `@type` | `syntax.class` |
| `@type.builtin` | `syntax.builtin` |
| `@type.definition` | `syntax.class-decl` |
| `@type.annotation` | `syntax.annotation` |
| `@annotation` | `syntax.annotation` |
| `@attribute` | `syntax.decorator` |
| `@variable` | `syntax.variable` |
| `@variable.parameter` | `syntax.parameter` |
| `@variable.builtin` | `syntax.self-parameter` |
| `@variable.member` | `syntax.attribute` |
| `@parameter` | `syntax.parameter` |
| `@property` | `syntax.attribute` |
| `@field` | `syntax.attribute` |
| `@module` | `syntax.module` |
| `@namespace` | `syntax.module` |
| `@operator` | `syntax.operator` |
| `@punctuation` | `syntax.punctuation` |
| `@punctuation.delimiter` | `syntax.punctuation` |
| `@punctuation.bracket` | `syntax.bracket` |
| `@punctuation.special` | `syntax.fstring-brace` |
| `@embedded` | `syntax.fstring-expression` |
| `@tag` | `syntax.tag` |
| `@tag.attribute` | `syntax.tag-attribute` |
| `@tag.delimiter` | `syntax.punctuation` |
| `@markup.link` | `syntax.link` |
| `@text.uri` | `syntax.link` |

### LSP semantic tokens

| Token type | Role |
|---|---|
| `namespace` | `syntax.module` |
| `type`, `class`, `enum`, `interface`, `struct`, `typeParameter` | `syntax.class`, or `syntax.class-decl` with the `declaration` or `definition` modifier |
| `parameter` | `syntax.parameter` (`self` and `cls` are told apart by their text: `syntax.self`) |
| `variable` | `syntax.variable`, or `syntax.constant` with `readonly` |
| `property`, `event` | `syntax.attribute` |
| `enumMember` | `syntax.constant` |
| `function` | `syntax.function-call`, or `syntax.function-decl` with `declaration` or `definition` |
| `method` | `syntax.method-call`, or `syntax.method-decl` with `declaration` or `definition` |
| `macro` | `syntax.function-call` |
| `keyword`, `modifier` | `syntax.keyword` |
| `comment` | `syntax.comment`, or `syntax.docstring` with `documentation` |
| `string`, `regexp` | `syntax.string` |
| `number` | `syntax.number` |
| `operator` | `syntax.operator` |
| `decorator` | `syntax.decorator` |

The modifier `defaultLibrary` on a function, method, class, type, variable,
struct or enum selects `syntax.builtin`. The modifier `deprecated` is an
overlay: `syntax.diagnostic-deprecated` is applied on top of the role.

Python names that need the text of the token (`__init__`, `__name__`,
`True`, `False`, `None`, `self`, `cls`, constants in `UPPER_CASE`) are decided
by the Python product and use `syntax.dunder`, `syntax.constant-builtin`,
`syntax.self` and `syntax.constant`.

## Not imported

- Icons and icon colour palettes (`icons`, `iconColorsOnSelection`).
- Component sizes and insets, borders, fonts of the UI, gradients, shadows
  and animation settings of Swing components: only the numbers listed under
  Metrics.
- Language specific attributes of other languages (`JAVA.*`, `GO.*`, ...),
  `CUSTOM_*`, `BLOCK_TERMINAL_*`, `JUPYTER.*`, `RAINBOW_*`, and every other key
  not listed above. They are counted by the first part of their name in the
  report (`unmapped by group`).
- `SELECTION_FOREGROUND`: Pyoxide keeps the syntax colours on selected text.
- Other `FILESTATUS_*` entries, and copies of them under a plugin specific
  prefix.

## The report

Every import returns a report with the sources that were read, the keys that
were mapped (and those that lost against an earlier key), the keys that were
not mapped with a count, approximations, the tokens taken from the base theme
and warnings (unknown parent, missing scheme, values that are not colours,
palette names that are not defined). `cargo run -p themes --example import --
<file|jar|zip|dir> [--context <path>] [--verbose] [--json] [--toml <dir>]`
prints it; `--toml` writes the imported themes as theme files.

Third-party themes are not stored in this repository. To check an importer
change against real files, point the example at a local copy (the
git-ignored folder `tests/fixtures/themes-local/` is the place for them).
