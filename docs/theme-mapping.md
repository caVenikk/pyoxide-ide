# Theme mapping

Pyoxide has its own theme model (crate `themes`). This document lists how the
keys of PyCharm colour schemes (`.icls`, scheme XML) and UI themes
(`.theme.json`) map to it, how a value is chosen when several keys compete, and
what is approximated or dropped. The key names are format identifiers and are
written as they appear in the files; no colour values of third-party themes are
recorded here. The tables below are checked against the code: a test fails when
a key or a role is missing from this file.

## The theme model

A theme assigns a value to every role of three kinds, plus fonts. The roles
are listed, with a description each, in `crates/platform/themes/src/roles.rs`.

- **UI roles** (`ui.*`): flat colours of the interface, the editor chrome,
  controls, icons, version control, diagnostics, the Run console and the
  terminal.
- **Text roles** (`syntax.*`): syntax tokens and editor decorations.
  Each has text attributes: foreground `fg`, background `bg`, `bold`, `italic`,
  `underline` (`single`, `double`, `wavy`, `dotted`, `dashed`, `thick`) with
  `underline-color`, `strikethrough` and a `border` colour. Unset attributes
  inherit from the editor defaults.
- **Metrics** (`metrics.*`): layout numbers in logical pixels (row height,
  gutter width, paddings, radii, ...).
- **Fonts** (`font.*`): UI, editor and terminal families (a list, first match
  wins), sizes, editor line spacing and ligatures.
- **Controls** (`controls` in `[theme]`): `new` (the default) or `classic`,
  the look of the controls that the reference draws differently for its New
  UI themes and for classic themes (themes not based on a New UI theme,
  such as most theme plugins): check boxes of 16 or 14 pixels, dialog
  buttons 28 or 24 high, chevrons or filled triangles as tree arrows. A file
  that does not set it takes it from the theme it `extends`.

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

A key of `[ui]`, `[syntax]` or `[metrics]` that names no role of this build,
an attribute of a text style and a field of `[theme]` that this build does not
know are skipped, so a file saved by a newer build that has more still loads
here: the known keys keep their values and the log gets one warning per file
that lists the skipped keys. Other errors (a bad value of a known key, broken
TOML, a newer `format`) still leave the file out.

The other direction: a self-contained file written by an older build lacks the
roles added since. They are filled, without a message, from the built-in theme
named by `base` in `[theme]` (an import records the theme it was made on), else
from the default theme of the file's kind (the base theme of an import for a
file with classic controls). Text styles are not filled: an absent one is
plain, as written.

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
| `pyoxide-dark` | Pyoxide Dark | original colours; base of an import (not under an islands theme) |
| `pyoxide-light` | Pyoxide Light | original colours; base of an import (not under an islands theme) |

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
   background before `Editor.background`. The `<colors>` option `BACKGROUND`
   is the deprecated form of that background and wins over it, as in the
   reference (`AbstractColorsScheme.readScheme`): when a scheme sets it to a
   colour, it replaces the background of the scheme's own `TEXT` entry, and a
   scheme without a `TEXT` entry gets one with black text on that colour (the
   report notes this, `text-from-background`). The rule works inside one
   scheme: the option of a parent scheme does not change a child's `TEXT`. The
   report lists such a background as `BACKGROUND` (scheme color). An empty
   value or one that is not a colour changes nothing.
6. **Defaults of the reference.** A role whose key the reference reads with a
   colour of its own for themes that do not set it takes that colour and is
   listed as an approximation: `ui.marker-popup-border` is `#4b4b4b` in a dark
   theme and `#cecece` in a light one (`LineStatusMarkerPopupPanel`);
   `ui.notification-text` is `#bbbbbb` in a dark theme (the `Darcula` base of a
   theme without a parent) and black in a light one; `ui.notification-border`
   is `#565a5c` (dark) or `#b2b2b2` (light) at 80 % opacity
   (`NotificationsManagerImpl.BORDER_COLOR`).
7. **Derived values.** A role no source supplies takes the value of its
   fallback role (tables below) when that role was imported or derived itself,
   and is listed as an approximation.
8. **Base theme.** Whatever is still missing comes from `pyoxide-dark` or
   `pyoxide-light` (chosen by `dark`; a scheme alone is dark when its editor
   background is dark, else by the name of its base scheme) and is listed in
   the report as taken from the base theme. A text role that a scheme defines
   replaces the base style completely; it is not merged with it. A theme
   whose `parentTheme` chain names `Islands Dark` or `Islands Light` (also
   through themes among the inputs) takes the bundled `islands-dark` or
   `islands-light` instead (by `dark`), since the parent chain it lacks
   would have set those roles. Under an islands base, steps 6 and 7 apply
   only to roles that follow a value of the colour scheme (the gutter follows
   the editor background); other roles take the islands value.

**Parent scheme.** The base schemes of the IDE (`Default`, `Darcula`, with the
Python attributes of `PythonDefault.xml` / `PythonDarcula.xml`) ship with the
importer (`crates/platform/themes/schemes/`, IntelliJ Platform Community,
Apache-2.0, `notices/themes-scheme-parent.md`). When a scheme names one of them
as its `parent_scheme` and the inputs do not hold it (pass the folder or archive
of an installed IDE as context to use that copy instead), the values the scheme
leaves unset come from the bundled scheme, as the reference resolves them:
every text role and every UI role that has a colour-scheme source
(`is_scheme_role`: the editor, the gutter, version control and diagnostic
colours, the terminal) is answered by the scheme chain first. The roles of the
UI theme (steps 6 to 8) keep the base theme of the import, also under an
islands base; fonts come only from the scheme files given. A `Darcula` among
the inputs wins over the bundled one, and its own `parent_scheme` `Default` is
the bundled one. A `parent_scheme` that is neither bundled nor among the inputs
is reported as before and the base theme fills the roles. The report notes the
use of a bundled scheme; the bundled files are not listed as sources and their
unused keys are not reported as unmapped. The one thing the product draws in
code is the Hint diagnostic: where the scheme leaves the information
attributes empty, `syntax.diagnostic-hint` is a grey dotted line
(`PyLspToolIntegrationProvider.getEnforcedTextAttributes`).

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
| `ui.segmented-selected` | `SegmentedButton.selectedButtonColor` | Background of the selected choice of a segmented button |
| `ui.segmented-selected-border` | `SegmentedButton.selectedStartBorderColor` | Outline of the selected choice of a segmented button |
| `ui.selection` | `List.selectionBackground`, `Tree.selectionBackground`, `Table.selectionBackground` | Background of the selected row in lists and trees |
| `ui.selection-text` | `List.selectionForeground`, `Tree.selectionForeground` | Text on the selected row |
| `ui.selection-inactive` | `List.selectionInactiveBackground`, `Tree.selectionInactiveBackground` | Selected row in a list that has no focus |
| `ui.selection-light` | `Plugins.lightSelectionBackground`, `Table.lightSelectionBackground` | Selected row of a list of rich rows (the projects of the project widget's popup and of the Welcome tab) |
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
| `ui.tab-selected-inactive` | `EditorTabs.inactiveUnderlinedTabBackground` | Selected tab of a tab strip without the focus: the editor tabs, a tool window's header tabs (`IslandsTabPainter`) |
| `ui.tab-underline-inactive` | `EditorTabs.inactiveUnderlinedTabBorderColor`, `EditorTabs.inactiveUnderlineColor`, `DefaultTabs.inactiveUnderlineColor` | Outline of that tab |
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
| `ui.run-widget-icon` | `RunWidget.iconColor` | Icons of the run widget's buttons other than Run and Debug, such as More Actions |
| `ui.run-widget-run-icon` | `RunWidget.runIconColor` | Run and Debug icons of the run widget (drawn as outlines in this colour) |
| `ui.run-widget-running-icon` | `RunWidget.runningIconColor` | Icons on the run widget's coloured buttons (Rerun while a process runs, Stop) |
| `ui.drag-and-drop-border` | `DragAndDrop.borderColor` | Border of the row that takes dragged files or folders (the reference's drop target rectangle) |
| `ui.drag-and-drop-row` | `DragAndDrop.rowBackground` | Background of the row that takes dragged files or folders |
| `ui.tree-text` | `Tree.foreground` | Text of tree rows, such as the pages of the Settings window and the Keymap and Run/Debug Configurations trees |
| `ui.tree-indent-guide` | `Tree.hash` | Indent guides of trees: the vertical lines below an open folder (the reference's "Show indent guides") |
| `ui.list-button-hover` | `List.Button.hoverBackground` | Background of a hovered button at the end of a popup row, such as Run in the run widget's dropdown |
| `ui.list-button-separator` | `List.Button.separatorColor` | Line before the buttons at the end of a popup row |

### Controls

The sources are keys of the icon colour palette (`icons` > `ColorPalette`
of a UI theme, see "Icon colours" below); the reference draws check boxes
as icons recoloured by it. A dark theme may write a key with `.Dark`
appended. A theme that sets none of a role's keys gets the colour of the
reference's check box icons for the look of its controls (`controls`):
those of the New UI themes, or the classic ones (`colorPaletteClassic`);
the report lists them as approximations. Tree arrows are chevron icons
for New UI controls and filled triangles in `ui.icon-gray-stroke` for
classic ones (as the reference shows them with classic theme plugins).

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.checkbox-background` | `Checkbox.Background.Default` (icon palette) | Fill of a check box that is off |
| `ui.checkbox-border` | `Checkbox.Border.Default` (icon palette) | Outline of a check box that is off |
| `ui.checkbox-background-selected` | `Checkbox.Background.Selected` (icon palette) | Fill of a check box that is on |
| `ui.checkbox-border-selected` | `Checkbox.Border.Selected` (icon palette) | Outline of a check box that is on |
| `ui.checkbox-check` | `Checkbox.Foreground.Selected` (icon palette) | Check mark of a check box that is on |
| `ui.checkbox-background-disabled` | `Checkbox.Background.Disabled` (icon palette) | Fill of a disabled check box |
| `ui.checkbox-border-disabled` | `Checkbox.Border.Disabled` (icon palette) | Outline of a disabled check box |
| `ui.checkbox-check-disabled` | `Checkbox.Foreground.Disabled` (icon palette) | Check mark of a disabled check box |

### Icon colours

The platform icons are the New UI icons; they are drawn with the colours
of a small palette. A UI theme recolours them with `icons` >
`ColorPalette`: every place where an icon uses the colour a key stands for
gets the theme's colour (`UITheme.toColorString`). Here each key is a
role; the base themes give every role the colour the key stands for, so
the icons keep their own colours, and a theme whose role differs has
that colour replaced in every icon. A key is also found when the theme
writes the colour it stands for instead of its name (`"#CED0D6":
"icon-default-stroke"` of Islands Dark is `Gray.Stroke`). `Blue.Solid`,
`Green.Solid` and `Red.Solid` stand for a colour in light themes only.

| Pyoxide role | Sources, in order | Colour it stands for, light / dark |
|---|---|---|
| `ui.icon-gray-fill` | `Gray.Fill` (icon palette) | `#EBECF0` / `#43454A` |
| `ui.icon-gray-secondary-stroke` | `Gray.SecondaryStroke` (icon palette) | `#A8ADBD` / `#6F737A` |
| `ui.icon-gray-stroke` | `Gray.Stroke` (icon palette) | `#6C707E` / `#CED0D6` |
| `ui.icon-blue-fill` | `Blue.Fill` (icon palette) | `#EDF3FF` / `#25324D` |
| `ui.icon-blue-solid` | `Blue.Solid` (icon palette) | `#4682FA` / none |
| `ui.icon-blue-stroke` | `Blue.Stroke` (icon palette) | `#3574F0` / `#548AF7` |
| `ui.icon-green-fill` | `Green.Fill` (icon palette) | `#F2FCF3` / `#253627` |
| `ui.icon-green-solid` | `Green.Solid` (icon palette) | `#55A76A` / none |
| `ui.icon-green-stroke` | `Green.Stroke` (icon palette) | `#208A3C` / `#57965C` |
| `ui.icon-red-fill` | `Red.Fill` (icon palette) | `#FFF7F7` / `#402929` |
| `ui.icon-red-solid` | `Red.Solid` (icon palette) | `#E55765` / none |
| `ui.icon-red-stroke` | `Red.Stroke` (icon palette) | `#DB3B4B` / `#DB5C5C` |
| `ui.icon-yellow-fill` | `Yellow.Fill` (icon palette) | `#FFFAEB` / `#3D3223` |
| `ui.icon-yellow-solid` | `Yellow.Solid` (icon palette) | `#FFAF0F` / `#F2C55C` |
| `ui.icon-yellow-stroke` | `Yellow.Stroke` (icon palette) | `#C27D04` / `#D6AE58` |
| `ui.icon-orange-fill` | `Orange.Fill` (icon palette) | `#FFF4EB` / `#45322B` |
| `ui.icon-orange-stroke` | `Orange.Stroke` (icon palette) | `#E56D17` / `#C77D55` |
| `ui.icon-purple-fill` | `Purple.Fill` (icon palette) | `#FAF5FF` / `#2F2936` |
| `ui.icon-purple-stroke` | `Purple.Stroke` (icon palette) | `#834DF0` / `#A571E6` |
| `ui.icon-teal-fill` | `Teal.Fill` (icon palette) | `#F2FCFC` / `#1D3838` |
| `ui.icon-teal-stroke` | `Teal.Stroke` (icon palette) | `#039BA1` / `#24A394` |

The palette keys of classic icons (`Actions.*`, `Objects.*`) stand for
colours the New UI icons do not use and are listed as unmapped.

### Editor

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.editor-background` | `TEXT` background (the scheme color `BACKGROUND` replaces it, see rule 5), `Editor.background` | Editor background |
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
| `ui.tearline` | `TEARLINE_COLOR` (scheme color) | Line under the titles of a diff's sides |
| `ui.whitespace` | `WHITESPACES` (scheme color) | Visible whitespace markers |
| `ui.soft-wrap-sign` | `SOFT_WRAP_SIGN_COLOR` (scheme color) | Signs at the soft wraps of a line |
| `ui.completion-popup` | `LOOKUP_COLOR` (scheme color) | Completion popup background |
| `ui.completion-selection` | `CompletionPopup.selectionBackground` | Selected completion item |
| `ui.completion-match` | `CompletionPopup.matchForeground` | Matched characters in completion items |
| `ui.documentation-popup` | `DOCUMENTATION_COLOR` (scheme color) | Documentation popup background |
| `ui.breakpoint` | `BREAKPOINT_ATTRIBUTES` background | Line with a breakpoint |
| `ui.execution-point` | `EXECUTIONPOINT_ATTRIBUTES` background | Line the debugger stopped on |
| `ui.execution-point-not-top` | `NOT_TOP_FRAME_ATTRIBUTES` background | Line of a frame the debugger shows other than the one it stopped in |
| `ui.bookmark` | `Bookmark.iconBackground` (UI theme or scheme color) | Bookmark marker |

### Version control

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.gutter-added` | `ADDED_LINES_COLOR` (scheme color) | Gutter marker of added lines |
| `ui.gutter-modified` | `MODIFIED_LINES_COLOR` (scheme color) | Gutter marker of modified lines |
| `ui.gutter-deleted` | `DELETED_LINES_COLOR` (scheme color) | Gutter marker of deleted lines |
| `ui.gutter-whitespace-modified` | `WHITESPACES_MODIFIED_LINES_COLOR` (scheme color) | Gutter marker of lines changed only in whitespace |
| `ui.gutter-ignored-added` | `IGNORED_ADDED_LINES_BORDER_COLOR` (scheme color) | Outline marker of added lines that are left out (another changelist) |
| `ui.gutter-ignored-modified` | `IGNORED_MODIFIED_LINES_BORDER_COLOR` (scheme color) | Outline marker of modified lines that are left out |
| `ui.gutter-ignored-deleted` | `IGNORED_DELETED_LINES_BORDER_COLOR` (scheme color) | Outline marker of deleted lines that are left out |
| `ui.stripe-added` | `DIFF_INSERTED` error stripe color | Error stripe mark of added lines |
| `ui.stripe-modified` | `DIFF_MODIFIED` error stripe color | Error stripe mark of modified lines |
| `ui.stripe-deleted` | `DIFF_DELETED` error stripe color | Error stripe mark of deleted lines |
| `ui.marker-popup-border` | `VersionControl.MarkerPopup.borderColor` | Border of the popup of a change marker |
| `ui.marker-popup-toolbar` | `VersionControl.MarkerPopup.Toolbar.background` | Toolbar background of the popup of a change marker |
| `ui.marker-popup-text` | `CHANGED_LINES_POPUP` (scheme color) | Background of the old text in the popup of a change marker |
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
| `ui.git-merged` | `FILESTATUS_MERGED` (scheme color) | File name colour: changed by a merge |

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
| `ui.terminal-background` | `CONSOLE_BACKGROUND_KEY` (scheme color) | Run console background |
| `ui.terminal-foreground` | `CONSOLE_NORMAL_OUTPUT` foreground | Run console default text |
| `ui.terminal-black` | `CONSOLE_BLACK_OUTPUT` foreground | ANSI colour 0 in the Run console |
| `ui.terminal-red` | `CONSOLE_RED_OUTPUT` foreground | ANSI colour 1 in the Run console |
| `ui.terminal-green` | `CONSOLE_GREEN_OUTPUT` foreground | ANSI colour 2 in the Run console |
| `ui.terminal-yellow` | `CONSOLE_YELLOW_OUTPUT` foreground | ANSI colour 3 in the Run console |
| `ui.terminal-blue` | `CONSOLE_BLUE_OUTPUT` foreground | ANSI colour 4 in the Run console |
| `ui.terminal-magenta` | `CONSOLE_MAGENTA_OUTPUT` foreground | ANSI colour 5 in the Run console |
| `ui.terminal-cyan` | `CONSOLE_CYAN_OUTPUT` foreground | ANSI colour 6 in the Run console |
| `ui.terminal-white` | `CONSOLE_GRAY_OUTPUT` foreground | ANSI colour 7 in the Run console |
| `ui.terminal-bright-black` | `CONSOLE_DARKGRAY_OUTPUT` foreground | ANSI colour 8 in the Run console |
| `ui.terminal-bright-red` | `CONSOLE_RED_BRIGHT_OUTPUT` foreground | ANSI colour 9 in the Run console |
| `ui.terminal-bright-green` | `CONSOLE_GREEN_BRIGHT_OUTPUT` foreground | ANSI colour 10 in the Run console |
| `ui.terminal-bright-yellow` | `CONSOLE_YELLOW_BRIGHT_OUTPUT` foreground | ANSI colour 11 in the Run console |
| `ui.terminal-bright-blue` | `CONSOLE_BLUE_BRIGHT_OUTPUT` foreground | ANSI colour 12 in the Run console |
| `ui.terminal-bright-magenta` | `CONSOLE_MAGENTA_BRIGHT_OUTPUT` foreground | ANSI colour 13 in the Run console |
| `ui.terminal-bright-cyan` | `CONSOLE_CYAN_BRIGHT_OUTPUT` foreground | ANSI colour 14 in the Run console |
| `ui.terminal-bright-white` | `CONSOLE_WHITE_OUTPUT` foreground | ANSI colour 15 in the Run console |
| `ui.terminal-default-background` | `BLOCK_TERMINAL_DEFAULT_BACKGROUND` (scheme color) | Terminal tool window background |
| `ui.terminal-default-foreground` | `BLOCK_TERMINAL_DEFAULT_FOREGROUND` (scheme color) | Terminal tool window default text |
| `ui.terminal-ansi-black` | `BLOCK_TERMINAL_BLACK` foreground, `BLOCK_TERMINAL_BLACK` background | Text of ANSI colour 0 in the terminal |
| `ui.terminal-ansi-red` | `BLOCK_TERMINAL_RED` foreground, `BLOCK_TERMINAL_RED` background | Text of ANSI colour 1 in the terminal |
| `ui.terminal-ansi-green` | `BLOCK_TERMINAL_GREEN` foreground, `BLOCK_TERMINAL_GREEN` background | Text of ANSI colour 2 in the terminal |
| `ui.terminal-ansi-yellow` | `BLOCK_TERMINAL_YELLOW` foreground, `BLOCK_TERMINAL_YELLOW` background | Text of ANSI colour 3 in the terminal |
| `ui.terminal-ansi-blue` | `BLOCK_TERMINAL_BLUE` foreground, `BLOCK_TERMINAL_BLUE` background | Text of ANSI colour 4 in the terminal |
| `ui.terminal-ansi-magenta` | `BLOCK_TERMINAL_MAGENTA` foreground, `BLOCK_TERMINAL_MAGENTA` background | Text of ANSI colour 5 in the terminal |
| `ui.terminal-ansi-cyan` | `BLOCK_TERMINAL_CYAN` foreground, `BLOCK_TERMINAL_CYAN` background | Text of ANSI colour 6 in the terminal |
| `ui.terminal-ansi-white` | `BLOCK_TERMINAL_WHITE` foreground, `BLOCK_TERMINAL_WHITE` background | Text of ANSI colour 7 in the terminal |
| `ui.terminal-ansi-bright-black` | `BLOCK_TERMINAL_BLACK_BRIGHT` foreground, `BLOCK_TERMINAL_BLACK_BRIGHT` background | Text of ANSI colour 8 in the terminal |
| `ui.terminal-ansi-bright-red` | `BLOCK_TERMINAL_RED_BRIGHT` foreground, `BLOCK_TERMINAL_RED_BRIGHT` background | Text of ANSI colour 9 in the terminal |
| `ui.terminal-ansi-bright-green` | `BLOCK_TERMINAL_GREEN_BRIGHT` foreground, `BLOCK_TERMINAL_GREEN_BRIGHT` background | Text of ANSI colour 10 in the terminal |
| `ui.terminal-ansi-bright-yellow` | `BLOCK_TERMINAL_YELLOW_BRIGHT` foreground, `BLOCK_TERMINAL_YELLOW_BRIGHT` background | Text of ANSI colour 11 in the terminal |
| `ui.terminal-ansi-bright-blue` | `BLOCK_TERMINAL_BLUE_BRIGHT` foreground, `BLOCK_TERMINAL_BLUE_BRIGHT` background | Text of ANSI colour 12 in the terminal |
| `ui.terminal-ansi-bright-magenta` | `BLOCK_TERMINAL_MAGENTA_BRIGHT` foreground, `BLOCK_TERMINAL_MAGENTA_BRIGHT` background | Text of ANSI colour 13 in the terminal |
| `ui.terminal-ansi-bright-cyan` | `BLOCK_TERMINAL_CYAN_BRIGHT` foreground, `BLOCK_TERMINAL_CYAN_BRIGHT` background | Text of ANSI colour 14 in the terminal |
| `ui.terminal-ansi-bright-white` | `BLOCK_TERMINAL_WHITE_BRIGHT` foreground, `BLOCK_TERMINAL_WHITE_BRIGHT` background | Text of ANSI colour 15 in the terminal |
| `ui.terminal-ansi-black-background` | `BLOCK_TERMINAL_BLACK` background, `BLOCK_TERMINAL_BLACK` foreground | Cell background of ANSI colour 0 in the terminal |
| `ui.terminal-ansi-red-background` | `BLOCK_TERMINAL_RED` background, `BLOCK_TERMINAL_RED` foreground | Cell background of ANSI colour 1 in the terminal |
| `ui.terminal-ansi-green-background` | `BLOCK_TERMINAL_GREEN` background, `BLOCK_TERMINAL_GREEN` foreground | Cell background of ANSI colour 2 in the terminal |
| `ui.terminal-ansi-yellow-background` | `BLOCK_TERMINAL_YELLOW` background, `BLOCK_TERMINAL_YELLOW` foreground | Cell background of ANSI colour 3 in the terminal |
| `ui.terminal-ansi-blue-background` | `BLOCK_TERMINAL_BLUE` background, `BLOCK_TERMINAL_BLUE` foreground | Cell background of ANSI colour 4 in the terminal |
| `ui.terminal-ansi-magenta-background` | `BLOCK_TERMINAL_MAGENTA` background, `BLOCK_TERMINAL_MAGENTA` foreground | Cell background of ANSI colour 5 in the terminal |
| `ui.terminal-ansi-cyan-background` | `BLOCK_TERMINAL_CYAN` background, `BLOCK_TERMINAL_CYAN` foreground | Cell background of ANSI colour 6 in the terminal |
| `ui.terminal-ansi-white-background` | `BLOCK_TERMINAL_WHITE` background, `BLOCK_TERMINAL_WHITE` foreground | Cell background of ANSI colour 7 in the terminal |
| `ui.terminal-ansi-bright-black-background` | `BLOCK_TERMINAL_BLACK_BRIGHT` background, `BLOCK_TERMINAL_BLACK_BRIGHT` foreground | Cell background of ANSI colour 8 in the terminal |
| `ui.terminal-ansi-bright-red-background` | `BLOCK_TERMINAL_RED_BRIGHT` background, `BLOCK_TERMINAL_RED_BRIGHT` foreground | Cell background of ANSI colour 9 in the terminal |
| `ui.terminal-ansi-bright-green-background` | `BLOCK_TERMINAL_GREEN_BRIGHT` background, `BLOCK_TERMINAL_GREEN_BRIGHT` foreground | Cell background of ANSI colour 10 in the terminal |
| `ui.terminal-ansi-bright-yellow-background` | `BLOCK_TERMINAL_YELLOW_BRIGHT` background, `BLOCK_TERMINAL_YELLOW_BRIGHT` foreground | Cell background of ANSI colour 11 in the terminal |
| `ui.terminal-ansi-bright-blue-background` | `BLOCK_TERMINAL_BLUE_BRIGHT` background, `BLOCK_TERMINAL_BLUE_BRIGHT` foreground | Cell background of ANSI colour 12 in the terminal |
| `ui.terminal-ansi-bright-magenta-background` | `BLOCK_TERMINAL_MAGENTA_BRIGHT` background, `BLOCK_TERMINAL_MAGENTA_BRIGHT` foreground | Cell background of ANSI colour 13 in the terminal |
| `ui.terminal-ansi-bright-cyan-background` | `BLOCK_TERMINAL_CYAN_BRIGHT` background, `BLOCK_TERMINAL_CYAN_BRIGHT` foreground | Cell background of ANSI colour 14 in the terminal |
| `ui.terminal-ansi-bright-white-background` | `BLOCK_TERMINAL_WHITE_BRIGHT` background, `BLOCK_TERMINAL_WHITE_BRIGHT` foreground | Cell background of ANSI colour 15 in the terminal |
| `ui.terminal-search-match` | `BLOCK_TERMINAL_SEARCH_ENTRY` background | Background of search matches in the terminal |
| `ui.terminal-search-match-text` | `BLOCK_TERMINAL_SEARCH_ENTRY` foreground | Text of search matches in the terminal |
| `ui.terminal-search-match-current` | `BLOCK_TERMINAL_CURRENT_SEARCH_ENTRY` background | Background of the current search match in the terminal |
| `ui.terminal-search-match-current-text` | `BLOCK_TERMINAL_CURRENT_SEARCH_ENTRY` foreground | Text of the current search match in the terminal |
| `ui.terminal-separator` | `BLOCK_TERMINAL_PROMPT_SEPARATOR_COLOR` (scheme color) | Line between the commands of the terminal |
| `ui.terminal-error-marker` | `BLOCK_TERMINAL_ERROR_BLOCK_STROKE_COLOR` (scheme color) | Marker beside a terminal command that failed |

The `terminal-*` roles without `ansi`, `default`, `search`, `separator` or
`error` colour the Run console (`CONSOLE_*`); the others colour the
Terminal tool window, which has separate text and cell background colours
for each ANSI colour (`BLOCK_TERMINAL_*` attributes: foreground for text,
background for cells, each standing in for the other when missing, as the
reference's `TerminalColorPalette` does). The reference falls back from
`BLOCK_TERMINAL_DEFAULT_*` to the console colours; a scheme without the
ANSI keys takes the console's ANSI colours here (listed as derived), where
the reference would take its parent scheme's terminal colours.

### Debugger

| Pyoxide role | Sources, in order | Used for |
|---|---|---|
| `ui.debugger-value-name` | `Debugger.Variables.valueForeground` | Names in the variables view of the debugger |
| `ui.debugger-type` | `Debugger.Variables.typeForeground` | Types in the variables view of the debugger: {list: 4} |
| `ui.debugger-changed-value` | `Debugger.Variables.changedValueForeground` | Names of values that changed since the debugger last stopped |
| `ui.debugger-collecting-data` | `Debugger.Variables.collectingDataForeground` | Collecting data while the debugger loads values |
| `ui.debugger-evaluating` | `Debugger.Variables.evaluatingExpressionForeground` | Evaluating while the debugger evaluates an expression |
| `ui.debugger-modifying-value` | `Debugger.Variables.modifyingValueForeground` | Modifying value while the debugger sets a value |
| `ui.debugger-exception` | `Debugger.Variables.exceptionForeground` | Exceptions in the variables view of the debugger |
| `ui.debugger-error-message` | `Debugger.Variables.errorMessageForeground` | Error messages in the variables view of the debugger |

Roles without a source are derived, then taken from the base theme.

### Fallback roles for UI roles

| Role | Takes the value of |
|---|---|
| `ui.surface-alt`, `ui.surface-raised`, `ui.input`, `ui.title-bar`, `ui.toolbar`, `ui.status-bar`, `ui.tool-window`, `ui.tab-bar`, `ui.notification`, `ui.editor-background` | `ui.surface` |
| `ui.tool-window-header`, `ui.tool-window-stripe` | `ui.tool-window` |
| `ui.tab-inactive` | `ui.tab-bar` |
| `ui.tab-active` | `ui.editor-background` |
| `ui.tab-selected-inactive` | `ui.tab-active` |
| `ui.tab-active-text`, `ui.tab-inactive-text` | `ui.text` |
| `ui.status-bar-text`, `ui.selection-text` | `ui.text` |
| `ui.selection-inactive`, `ui.selection-light` | `ui.selection` |
| `ui.border-input`, `ui.divider`, `ui.popup-border` | `ui.border` |
| `ui.segmented-selected` | `ui.pressed` |
| `ui.segmented-selected-border`, `ui.tab-underline-inactive` | `ui.border-input` |
| `ui.text-disabled`, `ui.shortcut` | `ui.text-muted` |
| `ui.text-link`, `ui.focus`, `ui.progress`, `ui.tab-underline`, `ui.completion-match` | `ui.accent` |
| `ui.scrollbar-hover` | `ui.scrollbar` |
| `ui.completion-popup`, `ui.documentation-popup` | `ui.surface-raised` |
| `ui.completion-selection` | `ui.selection` |
| `ui.gutter`, `ui.terminal-background` | `ui.editor-background` |
| `ui.line-number-active` | `ui.line-number` |
| `ui.caret`, `ui.terminal-foreground` | `ui.editor-text` |
| `ui.indent-guide-active`, `ui.tearline` | `ui.indent-guide` |
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
| `ui.tree-text` | `ui.text` |
| `ui.tree-indent-guide` | `ui.divider` |
| `ui.list-button-hover` | `ui.hover` |
| `ui.list-button-separator` | `ui.pressed` |
| `ui.notification-link` | `ui.text-link` |
| `ui.notification-icon-hover` | `ui.hover` |
| `ui.notification-more` | `ui.notification` |
| `ui.notification-more-text` | `ui.text-muted` |
| `ui.terminal-default-background` | `ui.terminal-background` |
| `ui.terminal-default-foreground` | `ui.terminal-foreground` |
| `ui.terminal-ansi-<colour>` (16 roles) | `ui.terminal-<colour>` |
| `ui.terminal-ansi-<colour>-background` (16 roles) | `ui.terminal-ansi-<colour>` |
| `ui.terminal-search-match` | `ui.search-match` |
| `ui.terminal-search-match-current` | `ui.search-match-active` |
| `ui.terminal-search-match-text`, `ui.terminal-search-match-current-text` | `ui.terminal-default-foreground` |
| `ui.terminal-separator` | `ui.divider` |
| `ui.terminal-error-marker` | `ui.danger` |
| `ui.execution-point-not-top` | `ui.current-line` |
| `ui.debugger-value-name` | `ui.text` |
| `ui.debugger-type`, `ui.debugger-collecting-data`, `ui.debugger-evaluating` | `ui.text-muted` |
| `ui.debugger-changed-value`, `ui.debugger-modifying-value` | `ui.accent` |
| `ui.debugger-exception`, `ui.debugger-error-message` | `ui.danger` |

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
| `syntax.debugger-inlined-values` | `DEBUGGER_INLINED_VALUES` | Values of variables shown after the lines while debugging |
| `syntax.debugger-inlined-values-modified` | `DEBUGGER_INLINED_VALUES_MODIFIED` | Values shown after the lines that changed since the last stop |
| `syntax.debugger-inlined-values-execution-line` | `DEBUGGER_INLINED_VALUES_EXECUTION_LINE` | Values shown after the line the program is suspended on |
| `syntax.debugger-evaluated-expression` | `EVALUATED_EXPRESSION_ATTRIBUTES` | The expression whose value the debugger shows under the mouse |
| `syntax.debugger-evaluated-expression-execution-line` | `EVALUATED_EXPRESSION_EXECUTION_LINE_ATTRIBUTES` | The expression whose value the debugger shows, on the line the program is suspended on |
| `syntax.reference-link` | `CTRL_CLICKABLE` | A value of the debugger after a line under the mouse, drawn as a link |

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
| `syntax.reference-link` | `syntax.link` |

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
that is not among the inputs is a note for `Default` and `Darcula` (the
bundled scheme answers, see "Parent scheme" above) and a warning for any other
name.

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

- Icon replacements by path (`icons` except `ColorPalette`) and the
  colours of icons on a selected row (`iconColorsOnSelection`).
- Component sizes and insets, borders, fonts of the UI, gradients, shadows
  and animation settings of Swing components: only the numbers listed under
  Metrics.
- Language specific attributes of other languages (`JAVA.*`, `GO.*`, ...),
  `CUSTOM_*`, `JUPYTER.*`, `RAINBOW_*`, the `BLOCK_TERMINAL_*` keys of the
  former block terminal (block backgrounds, the command and the generate
  command prompt) and every other key
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
