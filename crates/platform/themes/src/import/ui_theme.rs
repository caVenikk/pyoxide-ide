//! UI themes: the `.theme.json` files of theme plugins.

use std::collections::BTreeMap;

use serde_json::Value;

/// A parsed UI theme, not yet merged with its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct UiThemeData {
    pub name: String,
    pub dark: Option<bool>,
    pub author: Option<String>,
    /// `parentTheme`: an identifier (not a name) of the theme this one extends.
    pub parent: Option<String>,
    /// `editorScheme`: a path inside the plugin or the name of a scheme.
    pub editor_scheme: Option<String>,
    /// `colors`: palette name to colour text (or to another palette name).
    pub palette: BTreeMap<String, String>,
    /// `ui` flattened to leaf keys (`Button.default.foreground`); wildcard
    /// groups appear as `*.background`.
    pub ui: BTreeMap<String, Value>,
    /// Other top-level sections (icons, ...) with the number of leaves in each.
    pub other: BTreeMap<String, usize>,
    pub origin: String,
    /// Path inside the archive or directory, `/`-separated.
    pub entry_path: String,
}

/// Top-level keys that carry no colours and are not reported as unmapped.
const METADATA_KEYS: &[&str] = &[
    "name",
    "nameKey",
    "dark",
    "author",
    "parentTheme",
    "editorScheme",
    "id",
    "description",
    "version",
];

impl UiThemeData {
    pub fn parse(text: &str, origin: &str, entry_path: &str) -> Result<Self, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Value::Object(root) = value else {
            return Err("not a JSON object".into());
        };
        let is_string = |key: &str| root.get(key).is_some_and(Value::is_string);
        let looks_like_theme = matches!(root.get("ui"), Some(Value::Object(_)))
            || is_string("editorScheme")
            || is_string("parentTheme")
            || (is_string("name") && root.get("dark").is_some_and(Value::is_boolean));
        if !looks_like_theme {
            return Err("not a UI theme (no ui, editorScheme or parentTheme)".into());
        }
        let text_of = |key: &str| {
            root.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let mut data = UiThemeData {
            name: text_of("name").unwrap_or_else(|| file_stem(entry_path)),
            dark: root.get("dark").and_then(Value::as_bool),
            author: text_of("author"),
            parent: text_of("parentTheme"),
            editor_scheme: text_of("editorScheme"),
            palette: BTreeMap::new(),
            ui: BTreeMap::new(),
            other: BTreeMap::new(),
            origin: origin.to_owned(),
            entry_path: entry_path.to_owned(),
        };
        if let Some(Value::Object(colors)) = root.get("colors") {
            for (name, value) in colors {
                if let Some(text) = value.as_str() {
                    data.palette.insert(name.clone(), text.trim().to_owned());
                }
            }
        }
        if let Some(Value::Object(ui)) = root.get("ui") {
            for (key, value) in ui {
                flatten(key, value, &mut data.ui);
            }
        }
        for (key, value) in &root {
            if METADATA_KEYS.contains(&key.as_str()) || key == "ui" || key == "colors" {
                continue;
            }
            let mut leaves = BTreeMap::new();
            flatten(key, value, &mut leaves);
            data.other.insert(key.clone(), leaves.len().max(1));
        }
        Ok(data)
    }
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name).to_owned()
}

fn flatten(prefix: &str, value: &Value, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, inner) in map {
                flatten(&format!("{prefix}.{key}"), inner, out);
            }
        }
        _ => {
            out.insert(prefix.to_owned(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r##"{
        "name": "Sample", "dark": true, "author": "someone",
        "parentTheme": "Darcula", "editorScheme": "/themes/Sample.xml",
        "colors": { "bg": "#112233", "fg": "#ddeeff" },
        "ui": {
            "*": { "background": "bg", "arc": 6, "selectionBackground": { "os.default": "#445566" } },
            "Button": { "default": { "foreground": "fg" }, "arc": "8" },
            "Tree.rowHeight": 22
        },
        "icons": { "ColorPalette": { "a": "#000000", "b": "#ffffff" } },
        "UIDesigner": { "x": 1 }
    }"##;

    #[test]
    fn reads_metadata_palette_and_flat_keys() {
        let t = UiThemeData::parse(DOC, "s.theme.json", "themes/s.theme.json").unwrap();
        assert_eq!(t.name, "Sample");
        assert_eq!(t.dark, Some(true));
        assert_eq!(t.parent.as_deref(), Some("Darcula"));
        assert_eq!(t.editor_scheme.as_deref(), Some("/themes/Sample.xml"));
        assert_eq!(t.palette["bg"], "#112233");
        assert_eq!(t.ui["*.background"], Value::from("bg"));
        assert_eq!(t.ui["*.arc"], Value::from(6));
        assert_eq!(
            t.ui["*.selectionBackground.os.default"],
            Value::from("#445566")
        );
        assert_eq!(t.ui["Button.default.foreground"], Value::from("fg"));
        assert_eq!(t.ui["Tree.rowHeight"], Value::from(22));
        assert_eq!(t.other["icons"], 2);
        assert_eq!(t.other["UIDesigner"], 1);
    }

    #[test]
    fn name_defaults_to_the_file_stem() {
        let t = UiThemeData::parse(r#"{"ui": {}}"#, "x", "themes/dark_one.theme.json").unwrap();
        assert_eq!(t.name, "dark_one");
    }

    #[test]
    fn other_json_is_not_a_theme() {
        assert!(UiThemeData::parse("[]", "x", "x").is_err());
        assert!(UiThemeData::parse(r#"{"key": "a", "description": "b"}"#, "x", "x").is_err());
        assert!(UiThemeData::parse("{", "x", "x").is_err());
        // The key list shipped next to themes has a `ui` array.
        assert!(UiThemeData::parse(r#"{"name": "Keys", "ui": [{"key": "a"}]}"#, "x", "x").is_err());
    }
}
