# 0018. Keymaps: named keymaps, switching and import

- Status: accepted
- Date: 2026-10-16

## Context
ADR 0007 made keymaps Pyoxide's own TOML files that bind keystrokes to
command ids per key context, with two built-in keymaps (macOS, and Windows
and Linux) following the reference's defaults, and a test that presses
every binding. The keymap is fixed per platform. #40 asks for a choice on
first run and in Settings, a VS Code keymap, and import of the reference's
custom keymaps.

The reference names its keymaps "macOS" (file `Mac OS X 10.5+`),
"Windows" (`$default`), "GNOME", "KDE", "XWin", "VSCode" and "VSCode
(macOS)". A keymap file is XML with a `parent` and the actions whose
shortcuts differ from it: an action listed replaces all of the parent's
shortcuts of that action, an empty one removes them. Actions are
identified by the reference's action ids (`EditorDuplicate`,
`ActivateTerminalToolWindow`), not by context. The active keymap is in
`options/keymap.xml`, custom keymaps in `keymaps/*.xml` of the
configuration folder.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
platform/platform-resources/src/keymaps, plugins/keymaps/vscode-keymap/
resources/keymaps (`VSCode.xml`, `VSCode OSX.xml`), platform/platform-impl
keymap code (KeymapImpl, MacOSDefaultKeymap, KeymapManagerImpl).

## Options considered
1. Hand-write every keymap as a full TOML file. Simple to read, but the
   VS Code sets and imported keymaps would be copies that drift from their
   parents.
2. Keymaps as a parent plus overrides by command, with a table from the
   reference's action ids to Pyoxide commands, so the reference's XML (VS
   Code sets, the user's custom keymaps) is converted by one code path.
3. Keep the reference's XML as Pyoxide's format. Ties Pyoxide's commands
   and contexts to the reference's action model.

A Zed keymap was asked for: Zed's keymap files are GPL-3.0 and there is no
permissively licensed source for one.

## Decision
Option 2.

- **Built-in base keymaps** stay hand-written TOML (ADR 0007): "macOS" and
  "Windows". Linux uses "Windows" until the GNOME and KDE sets are added
  the same way.
- **Derived keymaps** are TOML with `version`, `name`, `parent` and
  overrides by command:

  ```toml
  version = 1
  name = "VSCode"
  parent = "Windows"

  [[override]]
  command = "editor.duplicate"
  keys = ["shift-alt-down"]

  [[override]]
  command = "editor.delete-line"
  keys = []            # removed
  ```

  An override replaces the command's bindings of the parent in every
  context the parent binds it in (globally when the parent does not bind
  it). Arguments are matched when the override names them. An override
  can name a `context` (`context = "Editor"`): it then replaces the
  command's bindings in that context only; the conversion of the
  reference's XML writes the contexts of the action table. A keystroke an
  override binds is taken from the parent's other commands in that
  context, so a keystroke stays bound once per context.
- **Action table**: `keymaps/actions/*.toml` (one file per menu) in the
  workbench maps the reference's action ids to Pyoxide commands (with
  arguments and a context where one action is several commands). It is
  written by hand from the Community keymap files; actions Pyoxide does not
  have are listed as unmapped.
  An action can have `use-shortcut-of = "<action id>"`: it has the
  shortcuts of that action unless the keymap lists it itself (the
  editor's `EditorCopy` uses `$Copy`'s, so a keymap that lists only
  `$Copy` binds both). Such an action is not shown in the Keymap page's
  tree, as in the reference.
- **VS Code keymaps**: the Community files `VSCode.xml` and
  `VSCode OSX.xml` are kept unmodified as data (NOTICE entry) and converted
  through the table when selected: "VSCode" on top of "Windows",
  "VSCode (macOS)" on top of "macOS".
- **Import**: the reference's active keymap (`keymap.xml`) and its custom
  keymaps (`keymaps/*.xml`, parent chain followed to a built-in one) are
  converted once by the import of ADR 0017 into `keymaps/<name>.toml` in
  the configuration folder; the report lists the shortcuts of unmapped
  actions. A keymap file can also be imported by command
  (`settings.import-keymap {path}`).
- **Choice and switching**: `[keymap] name` in `settings.toml` (absent: the
  platform's default). Switching clears GPUI's key bindings and binds the
  resolved keymap, its double taps and mouse buttons; menus, tooltips and
  the Keymap page read shortcuts from the active keymap. It applies on
  Apply in Settings and at once from the gear menu's Keymap submenu
  (`settings.switch-keymap {name?}`).
- **Zed**: not shipped.
- **Tests**: the built-in keymaps keep the keystroke test of ADR 0007.
  Derived keymaps get a resolution test: every binding resolves to a
  registered command, no keystroke is bound twice in one context, and the
  list of unmapped actions is a checked expectation, so a new mapping
  shows up in review.

## Consequences
- The action table becomes a maintained file: a new command that has a
  reference action gets a row.
- Keymaps are read-only in the first version; the Keymap page shows the
  resolved tree with search by name and by shortcut. Editing shortcuts,
  Duplicate and Restore Defaults come later and write derived keymaps.
- The reference's control/meta swap for keymaps inherited on macOS is not
  needed: a derived keymap names a Pyoxide base keymap as its parent.
- A Zed keymap needs a permissively licensed source or the maintainer's
  decision to write one from public documentation of its bindings.
