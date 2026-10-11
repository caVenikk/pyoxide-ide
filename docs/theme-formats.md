# PyCharm theme formats

What the importer must read, observed on real files: the Dracula 2.3.1 and
Monokai Pro 2.2.5 theme plugins and the themes bundled with PyCharm 2026.1.
Third-party theme files are not part of this repository (they live in the
git-ignored `tests/fixtures/themes-local/`); open tests use synthetic or openly
licensed themes.

## Two layers
1. **UI theme** `*.theme.json` (JSON): `name`, `dark`, `author`, `parentTheme`,
   `editorScheme`, `colors` (palette name -> hex), `ui` (component groups ->
   keys), `icons` (`ColorPalette` for recolouring SVG icons).
2. **Editor colour scheme** (XML; `.xml` inside plugins, `.icls` for user
   schemes): `<scheme name version parent_scheme>` with `<colors>` and
   `<attributes>` (around 1,150 `<option>` entries). Keys look like
   `PY.BUILTIN_NAME`, `PY.ANNOTATION`, `PY.KEYWORD_ARGUMENT`,
   `PY.SELF_PARAMETER`, `DEFAULT_*`.

## Inheritance chains (resolve on import)
- Scheme: `Dracula Colorful` / `Monokai Pro` / `Islands Dark` -> `Darcula` ->
  `Default` (base schemes live in `DefaultColorSchemesManager.xml`). Light
  schemes inherit from `Default`.
- UI theme: `Islands Dracula Colorful` -> `Islands Dark` -> `ExperimentalDark`
  (`expUI_dark.theme.json`, whose `name` is "Dark") -> `Darcula`.
  `parentTheme` holds an **id**, not a `name`; the id-to-file registry is not in
  the theme files, so the importer needs its own table (start with
  ExperimentalDark -> expUI_dark, ExperimentalLight -> expUI_light,
  ExperimentalLightWithLightHeader -> expUI_light_with_light_header, Darcula,
  IntelliJ). An unknown parent produces a warning in the import report.
- Some themes (Monokai Pro) have no `parentTheme` and define everything explicitly.

## UI keys
Values in `ui` are a hex colour, **a name from `colors`**
(`"background": "primaryBackground"`), or a number/string (`"arc": "7"`). The
group `"*"` provides defaults for all components. Group and key names refer to
Swing components (`Button.background` and so on): import the ones that have an
equivalent widget here and list the rest in the report.

## Details the importer relies on
- Theme files in plugins are usually `*.theme.json`, but some plugins name them
  `*.json` inside a `themes/` folder. A JSON file there counts as a UI theme when
  it has a `ui` object, an `editorScheme` or `parentTheme` string, or a `name` and
  a boolean `dark`. The files under `themes/metadata/` are lists of known UI
  keys, not themes.
- `editorScheme` is either a path inside the plugin (`/themes/Name.xml`) or the
  name of a scheme (`Darcula`). A theme without it uses the default scheme of
  its brightness.
- The base schemes `Default` and `Darcula` are defined in
  `DefaultColorSchemesManager.xml` of the IDE, which holds several `<scheme>`
  elements. The importer reads it when it is passed as context (the importer
  never ships it); without it, values a scheme leaves to its parent are filled
  from the built-in theme and the report says so.
- Colours: scheme XML writes `RRGGBB` or `RRGGBBAA` without `#` (shorter values
  are hex numbers, `0` is black); `.theme.json` writes `#RRGGBB` or `#RRGGBBAA`
  or a name from `colors`.
- In `ui`, the group `*` holds wildcards (`*.background`); a value can be an
  object of platform variants (`os.default`, `os.windows`, ...).
- Scheme attributes may be written as `baseAttributes="OTHER_KEY"` instead of
  a `<value>`; an empty `<value />` replaces the parent's entry.
- `EFFECT_TYPE` values seen: 0 box, 1 line, 2 wave, 3 strikeout, 4 bold line,
  5 bold dotted.

The key by key mapping is in [theme-mapping.md](theme-mapping.md).

## Acceptance criteria
- Load a UI theme and its editor scheme via `editorScheme`.
- Resolve both chains; import partially and report when a parent is missing.
- From plugin archives read only data files (`themes/*`, `*.xml`, `*.icls`,
  `*.theme.json`); never `.class`.
- Produce a per-theme report: imported / not imported.
- Mapping of PyCharm attributes (`PY.*`, `DEFAULT_*`) to tree-sitter and LSP
  tokens is documented in `docs/theme-mapping.md`.
