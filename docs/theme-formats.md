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

## Acceptance criteria
- Load a UI theme and its editor scheme via `editorScheme`.
- Resolve both chains; import partially and report when a parent is missing.
- From plugin archives read only data files (`themes/*`, `*.xml`, `*.icls`,
  `*.theme.json`); never `.class`.
- Produce a per-theme report: imported / not imported.
- Mapping of PyCharm attributes (`PY.*`, `DEFAULT_*`) to tree-sitter and LSP
  tokens is documented in `docs/theme-mapping.md`.
