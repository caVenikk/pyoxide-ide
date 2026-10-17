- crates/platform/themes/themes/islands-dark.toml, islands-light.toml
  Colours of the New UI "Islands" themes of the IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @
  f5a0bddb9cee9135c700de9a67534e11cdf61c07. Files:
  platform/platform-resources/src/themes/islands/ManyIslandsDark.theme.json,
  ManyIslandsLight.theme.json, IslandSchemeDark.xml;
  platform/platform-resources/src/themes/expUI/expUI_dark.theme.json,
  expUI_light.theme.json, expUI_light_with_light_header.theme.json,
  expUI_lightScheme.xml; platform/platform-resources/src/themes/darcula.theme.json,
  intellijlaf.theme.json; platform/platform-resources/src/DefaultColorSchemesManager.xml;
  python/pluginResources/colorSchemes/PythonDarcula.xml, PythonDefault.xml.
  Changes: converted by the importer of the themes crate into Pyoxide's theme
  model (only the mapped keys, see docs/theme-mapping.md), renamed ids;
  regenerated with `cargo run -p themes --example builtin_islands`.
