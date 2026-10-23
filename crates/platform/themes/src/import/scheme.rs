//! Editor colour schemes: the `<scheme>` XML of `.icls` files and of the
//! scheme files inside theme plugins.

use std::collections::BTreeMap;

use crate::color::Rgba;

use super::xml::{self, Element};

/// The `<colors>` option that older schemes use for the editor background.
/// It is deprecated in favour of the background of [`TEXT`].
pub(crate) const BACKGROUND_OPTION: &str = "BACKGROUND";

/// The attribute key of plain editor text: the editor's default colours.
pub(crate) const TEXT: &str = "TEXT";

/// One text attribute entry of a scheme. Every field holds the raw text of
/// the option; empty and absent options are `None` ("inherit").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribute {
    /// `baseAttributes`: the key whose attributes this entry starts from.
    pub base: Option<String>,
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub font_type: Option<String>,
    pub effect_color: Option<String>,
    pub effect_type: Option<String>,
    pub error_stripe: Option<String>,
    /// What the scheme's `<colors>` option `BACKGROUND` did to this entry
    /// (only the `TEXT` entry is touched).
    pub background_option: BackgroundOption,
}

/// How the deprecated `<colors>` option `BACKGROUND` shaped an attribute
/// entry, see [`Scheme::apply_background_option`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackgroundOption {
    /// The entry is as the file wrote it.
    #[default]
    Unused,
    /// The option replaced the background of the scheme's own entry.
    Replaced,
    /// The scheme had no entry; one was made with black text on the
    /// option's colour.
    Created,
}

impl Attribute {
    /// Own fields win; unset ones come from `under`.
    fn over(&self, under: &Attribute) -> Attribute {
        let pick = |own: &Option<String>, other: &Option<String>| own.clone().or(other.clone());
        Attribute {
            base: self.base.clone(),
            foreground: pick(&self.foreground, &under.foreground),
            background: pick(&self.background, &under.background),
            font_type: pick(&self.font_type, &under.font_type),
            effect_color: pick(&self.effect_color, &under.effect_color),
            effect_type: pick(&self.effect_type, &under.effect_type),
            error_stripe: pick(&self.error_stripe, &under.error_stripe),
            background_option: if self.background.is_some() {
                self.background_option
            } else {
                under.background_option
            },
        }
    }
}

/// Font options of a scheme (all optional).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemeFonts {
    pub editor_name: Option<String>,
    pub editor_size: Option<f32>,
    pub line_spacing: Option<f32>,
    pub ligatures: Option<bool>,
    pub console_name: Option<String>,
    pub console_size: Option<f32>,
}

/// A parsed colour scheme, not yet merged with its parent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub version: Option<String>,
    /// `parent_scheme`: the name of the scheme this one inherits from.
    pub parent: Option<String>,
    pub default_scheme: bool,
    /// Where the scheme was read from, for messages.
    pub origin: String,
    /// Path inside the archive or directory, `/`-separated, for lookups by
    /// the `editorScheme` path of a UI theme.
    pub entry_path: String,
    /// `<colors>`: option name to raw value (possibly empty).
    pub colors: BTreeMap<String, String>,
    /// `<attributes>`: key to entry.
    pub attributes: BTreeMap<String, Attribute>,
    /// Options directly under `<scheme>` that are not fonts.
    pub other_options: Vec<String>,
    pub fonts: SchemeFonts,
    /// Additional text attributes of a language for the base scheme named
    /// `name` (a `<list>` file such as the Python plugin's attributes for
    /// `Darcula`), not a scheme of its own: it is merged below that scheme.
    pub additional: bool,
    /// The copy of a base scheme (`Default`, `Darcula`) that ships with the
    /// crate, used when a scheme names it as its `parent_scheme` and the
    /// inputs do not hold it (see `import::bundled`).
    pub bundled: bool,
}

impl Scheme {
    /// Parses a document and returns every `<scheme>` in it (a `.icls` file
    /// holds one, the application defaults file holds several).
    pub fn parse(text: &str, origin: &str, entry_path: &str) -> Result<Vec<Scheme>, String> {
        let root = xml::parse(text)?;
        let mut nodes = Vec::new();
        root.descendants_named("scheme", &mut nodes);
        if nodes.is_empty() {
            if let Some(list) = additional_list(&root) {
                return Self::parse_additional(list, origin, entry_path).map(|s| vec![s]);
            }
            return Err("no <scheme> element".into());
        }
        Ok(nodes
            .into_iter()
            .map(|node| Scheme::from_element(node, origin, entry_path))
            .collect())
    }

    /// A `<list>` of attribute options that a language adds to a base
    /// scheme. The base scheme is named by the end of the file name, as the
    /// language plugins name these files (`...Darcula.xml`, `...Default.xml`).
    fn parse_additional(list: &Element, origin: &str, entry_path: &str) -> Result<Scheme, String> {
        let file = entry_path.rsplit('/').next().unwrap_or(entry_path);
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
        let target = ["Darcula", "Default"]
            .into_iter()
            .find(|base| stem.ends_with(base) && stem.len() > base.len())
            .ok_or_else(|| {
                format!(
                    "additional attributes for an unknown scheme (file `{file}`; \
                     expected a name ending in Darcula or Default)"
                )
            })?;
        let mut scheme = Scheme {
            name: target.to_owned(),
            origin: origin.to_owned(),
            entry_path: entry_path.to_owned(),
            additional: true,
            ..Scheme::default()
        };
        for option in list.children_named("option") {
            if let Some(name) = option.attr("name")
                && let Some(attribute) = parse_attribute(option)
            {
                scheme.attributes.insert(name.to_owned(), attribute);
            }
        }
        Ok(scheme)
    }

    fn from_element(node: &Element, origin: &str, entry_path: &str) -> Scheme {
        let mut scheme = Scheme {
            name: node.attr("name").unwrap_or_default().to_owned(),
            version: node.attr("version").map(str::to_owned),
            parent: node
                .attr("parent_scheme")
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
            default_scheme: node.attr("default_scheme") == Some("true"),
            origin: origin.to_owned(),
            entry_path: entry_path.to_owned(),
            ..Scheme::default()
        };
        for colors in node.children_named("colors") {
            for option in colors.children_named("option") {
                if let Some(name) = option.attr("name") {
                    let value = option.attr("value").unwrap_or_default().trim().to_owned();
                    scheme.colors.insert(name.to_owned(), value);
                }
            }
        }
        for attributes in node.children_named("attributes") {
            for option in attributes.children_named("option") {
                if let Some(name) = option.attr("name")
                    && let Some(attribute) = parse_attribute(option)
                {
                    scheme.attributes.insert(name.to_owned(), attribute);
                }
            }
        }
        for option in node.children_named("option") {
            scheme.read_font_option(option, FontScope::Editor);
        }
        for font in node.children_named("font") {
            for option in font.children_named("option") {
                scheme.read_font_option(option, FontScope::Editor);
            }
        }
        for font in node.children_named("console-font") {
            for option in font.children_named("option") {
                scheme.read_font_option(option, FontScope::Console);
            }
        }
        scheme.apply_background_option();
        scheme
    }

    /// `<colors><option name="BACKGROUND">` is the deprecated form of the
    /// `TEXT` background, and it wins over it: as `AbstractColorsScheme.readScheme`
    /// does once a `<scheme>` is read, a colour in it replaces the background
    /// of the scheme's own `TEXT` entry, and a scheme without one gets a
    /// `TEXT` entry with black text on that colour. The rule works inside one
    /// scheme: the option of a parent does not reach a child's `TEXT`. An
    /// empty or unreadable value changes nothing.
    fn apply_background_option(&mut self) {
        let Some(value) = self
            .colors
            .get(BACKGROUND_OPTION)
            .filter(|v| Rgba::parse_scheme(v).is_ok())
            .cloned()
        else {
            return;
        };
        let text = self
            .attributes
            .entry(TEXT.to_owned())
            .or_insert_with(|| Attribute {
                foreground: Some("000000".to_owned()),
                background_option: BackgroundOption::Created,
                ..Attribute::default()
            });
        text.background = Some(value);
        if text.background_option == BackgroundOption::Unused {
            text.background_option = BackgroundOption::Replaced;
        }
    }

    fn read_font_option(&mut self, option: &Element, scope: FontScope) {
        let (Some(name), value) = (option.attr("name"), option.attr("value")) else {
            return;
        };
        let value = value.map(str::trim).filter(|v| !v.is_empty());
        let fonts = &mut self.fonts;
        match (scope, name) {
            (FontScope::Editor, "EDITOR_FONT_NAME") => fonts.editor_name = value.map(str::to_owned),
            (FontScope::Editor, "EDITOR_FONT_SIZE") => {
                fonts.editor_size = value.and_then(|v| v.parse().ok())
            }
            (FontScope::Console, "EDITOR_FONT_NAME") | (_, "CONSOLE_FONT_NAME") => {
                fonts.console_name = value.map(str::to_owned)
            }
            (FontScope::Console, "EDITOR_FONT_SIZE") | (_, "CONSOLE_FONT_SIZE") => {
                fonts.console_size = value.and_then(|v| v.parse().ok())
            }
            (_, "LINE_SPACING") => fonts.line_spacing = value.and_then(|v| v.parse().ok()),
            (_, "EDITOR_LIGATURES") => fonts.ligatures = value.map(|v| v == "true"),
            _ => self.other_options.push(name.to_owned()),
        }
    }
}

#[derive(Clone, Copy)]
enum FontScope {
    Editor,
    Console,
}

/// The root `<list>` element of an additional attributes file.
fn additional_list(root: &Element) -> Option<&Element> {
    if root.name == "list" {
        return Some(root);
    }
    root.child("list")
}

fn parse_attribute(option: &Element) -> Option<Attribute> {
    let base = option
        .attr("baseAttributes")
        .filter(|b| !b.is_empty())
        .map(str::to_owned);
    let value = option.child("value");
    if base.is_none() && value.is_none() {
        return None;
    }
    let mut attribute = Attribute {
        base,
        ..Attribute::default()
    };
    if let Some(value) = value {
        for field in value.children_named("option") {
            let Some(name) = field.attr("name") else {
                continue;
            };
            let text = field
                .attr("value")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            match name {
                "FOREGROUND" => attribute.foreground = text,
                "BACKGROUND" => attribute.background = text,
                "FONT_TYPE" => attribute.font_type = text,
                "EFFECT_COLOR" => attribute.effect_color = text,
                "EFFECT_TYPE" => attribute.effect_type = text,
                "ERROR_STRIPE_COLOR" => attribute.error_stripe = text,
                _ => {}
            }
        }
    }
    Some(attribute)
}

/// A scheme and its ancestors that are available, nearest first.
#[derive(Debug, Clone, Default)]
pub(crate) struct SchemeChain<'a> {
    pub layers: Vec<&'a Scheme>,
}

impl SchemeChain<'_> {
    /// Raw value of a `<colors>` option. An option that is present but empty
    /// means "no colour" and hides the parents' value.
    pub fn color(&self, key: &str) -> Option<&str> {
        self.color_with(key, true)
    }

    /// [`SchemeChain::color`], optionally without the bundled base schemes.
    pub fn color_with(&self, key: &str, bundled: bool) -> Option<&str> {
        for layer in self.layers.iter().filter(|l| bundled || !l.bundled) {
            if let Some(value) = layer.colors.get(key) {
                return Some(value.as_str()).filter(|v| !v.is_empty());
            }
        }
        None
    }

    /// The attribute entry for `key` with `baseAttributes` followed.
    pub fn attribute(&self, key: &str) -> Option<Attribute> {
        self.attribute_with(key, true)
    }

    /// [`SchemeChain::attribute`], optionally without the bundled base
    /// schemes.
    pub fn attribute_with(&self, key: &str, bundled: bool) -> Option<Attribute> {
        self.attribute_at(key, 0, bundled)
    }

    fn attribute_at(&self, key: &str, depth: usize, bundled: bool) -> Option<Attribute> {
        const MAX_BASE_DEPTH: usize = 8;
        if depth > MAX_BASE_DEPTH {
            return None;
        }
        for layer in self.layers.iter().filter(|l| bundled || !l.bundled) {
            if let Some(own) = layer.attributes.get(key) {
                return Some(match own.base.as_deref() {
                    Some(base) => match self.attribute_at(base, depth + 1, bundled) {
                        Some(under) => own.over(&under),
                        None => own.clone(),
                    },
                    None => own.clone(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<scheme name="Sample" version="142" parent_scheme="Base">
  <option name="LINE_SPACING" value="1.25" />
  <option name="EDITOR_FONT_SIZE" value="15" />
  <option name="EDITOR_FONT_NAME" value="Example Mono" />
  <option name="SOMETHING_ELSE" value="x" />
  <font>
    <option name="EDITOR_FONT_NAME" value="Example Mono" />
    <option name="EDITOR_FONT_SIZE" value="15" />
  </font>
  <console-font>
    <option name="EDITOR_FONT_NAME" value="Console Mono" />
    <option name="EDITOR_FONT_SIZE" value="12" />
  </console-font>
  <colors>
    <option name="CARET_COLOR" value="0" />
    <option name="SELECTION_FOREGROUND" value="" />
    <option name="GUTTER_BACKGROUND" value="102030" />
  </colors>
  <attributes>
    <option name="DEFAULT_KEYWORD">
      <value>
        <option name="FOREGROUND" value="aabbcc" />
        <option name="FONT_TYPE" value="1" />
        <option name="BACKGROUND" />
      </value>
    </option>
    <option name="PY.DECORATOR" baseAttributes="DEFAULT_KEYWORD" />
    <option name="PY.ANNOTATION" baseAttributes="DEFAULT_KEYWORD">
      <value><option name="FOREGROUND" value="112233" /></value>
    </option>
    <option name="EXPLICITLY_EMPTY"><value /></option>
    <option name="IGNORED_BARE" />
  </attributes>
</scheme>"#;

    fn sample() -> Scheme {
        Scheme::parse(DOC, "sample.icls", "sample.icls")
            .unwrap()
            .remove(0)
    }

    #[test]
    fn reads_header_colors_and_attributes() {
        let s = sample();
        assert_eq!(s.name, "Sample");
        assert_eq!(s.parent.as_deref(), Some("Base"));
        assert_eq!(s.colors["CARET_COLOR"], "0");
        assert_eq!(s.colors["SELECTION_FOREGROUND"], "");
        let kw = &s.attributes["DEFAULT_KEYWORD"];
        assert_eq!(kw.foreground.as_deref(), Some("aabbcc"));
        assert_eq!(kw.font_type.as_deref(), Some("1"));
        assert_eq!(kw.background, None);
        assert!(s.attributes.contains_key("EXPLICITLY_EMPTY"));
        assert!(!s.attributes.contains_key("IGNORED_BARE"));
    }

    #[test]
    fn reads_fonts() {
        let s = sample();
        assert_eq!(s.fonts.editor_name.as_deref(), Some("Example Mono"));
        assert_eq!(s.fonts.editor_size, Some(15.0));
        assert_eq!(s.fonts.line_spacing, Some(1.25));
        assert_eq!(s.fonts.console_name.as_deref(), Some("Console Mono"));
        assert_eq!(s.fonts.console_size, Some(12.0));
        assert_eq!(s.other_options, ["SOMETHING_ELSE"]);
    }

    #[test]
    fn base_attributes_are_followed_and_overridden() {
        let s = sample();
        let chain = SchemeChain { layers: vec![&s] };
        let deco = chain.attribute("PY.DECORATOR").unwrap();
        assert_eq!(deco.foreground.as_deref(), Some("aabbcc"));
        assert_eq!(deco.font_type.as_deref(), Some("1"));
        let ann = chain.attribute("PY.ANNOTATION").unwrap();
        assert_eq!(ann.foreground.as_deref(), Some("112233"));
        assert_eq!(ann.font_type.as_deref(), Some("1"));
        assert!(chain.attribute("MISSING").is_none());
    }

    #[test]
    fn base_attribute_cycles_terminate() {
        let doc = r#"<scheme name="c"><attributes>
            <option name="A" baseAttributes="B" />
            <option name="B" baseAttributes="A" />
        </attributes></scheme>"#;
        let s = Scheme::parse(doc, "c", "c").unwrap().remove(0);
        let chain = SchemeChain { layers: vec![&s] };
        assert!(chain.attribute("A").is_some());
    }

    #[test]
    fn child_layers_override_and_empty_colour_hides_the_parent() {
        let parent = Scheme::parse(
            r#"<scheme name="p"><colors><option name="A" value="111111"/><option name="B" value="222222"/></colors>
               <attributes><option name="K"><value><option name="FOREGROUND" value="333333"/></value></option></attributes></scheme>"#,
            "p",
            "p",
        )
        .unwrap()
        .remove(0);
        let child = Scheme::parse(
            r#"<scheme name="c" parent_scheme="p"><colors><option name="A" value="" /></colors>
               <attributes><option name="K"><value /></option></attributes></scheme>"#,
            "c",
            "c",
        )
        .unwrap()
        .remove(0);
        let chain = SchemeChain {
            layers: vec![&child, &parent],
        };
        assert_eq!(chain.color("A"), None);
        assert_eq!(chain.color("B"), Some("222222"));
        // The child's empty entry replaces the parent's entry entirely.
        assert_eq!(chain.attribute("K").unwrap().foreground, None);
    }

    fn one(doc: &str) -> Scheme {
        Scheme::parse(doc, "x", "x").unwrap().remove(0)
    }

    #[test]
    fn the_background_option_replaces_the_text_background() {
        let s = one(
            r#"<scheme name="s"><colors><option name="BACKGROUND" value="101010"/></colors>
            <attributes><option name="TEXT"><value>
              <option name="FOREGROUND" value="e0e0e0"/><option name="BACKGROUND" value="303030"/>
              <option name="EFFECT_TYPE" value="5"/>
            </value></option></attributes></scheme>"#,
        );
        let text = &s.attributes[TEXT];
        assert_eq!(text.background.as_deref(), Some("101010"));
        assert_eq!(text.foreground.as_deref(), Some("e0e0e0"));
        assert_eq!(text.effect_type.as_deref(), Some("5"));
        assert_eq!(text.background_option, BackgroundOption::Replaced);
        // The option stays a colour of the scheme.
        assert_eq!(s.colors[BACKGROUND_OPTION], "101010");
    }

    #[test]
    fn the_background_option_without_text_makes_black_text() {
        // `<colors>` after `<attributes>`: the order does not matter.
        let s = one(
            r#"<scheme name="s"><attributes/><colors><option name="BACKGROUND" value="f0f0f0"/></colors></scheme>"#,
        );
        let text = &s.attributes[TEXT];
        assert_eq!(text.foreground.as_deref(), Some("000000"));
        assert_eq!(text.background.as_deref(), Some("f0f0f0"));
        assert_eq!(text.background_option, BackgroundOption::Created);
    }

    #[test]
    fn an_empty_or_broken_background_option_changes_nothing() {
        for value in ["", "not a colour"] {
            let s = one(&format!(
                r#"<scheme name="s"><colors><option name="BACKGROUND" value="{value}"/></colors>
                <attributes><option name="TEXT"><value><option name="BACKGROUND" value="303030"/></value></option></attributes></scheme>"#
            ));
            let text = &s.attributes[TEXT];
            assert_eq!(text.background.as_deref(), Some("303030"), "{value:?}");
            assert_eq!(text.background_option, BackgroundOption::Unused);
        }
        let none = one(
            r#"<scheme name="s"><colors><option name="BACKGROUND" value=""/></colors></scheme>"#,
        );
        assert!(!none.attributes.contains_key(TEXT));
    }

    #[test]
    fn the_background_option_stays_inside_its_scheme() {
        let parent = one(
            r#"<scheme name="p"><colors><option name="BACKGROUND" value="111111"/></colors></scheme>"#,
        );
        let child = one(r#"<scheme name="c" parent_scheme="p">
            <attributes><option name="TEXT"><value><option name="BACKGROUND" value="222222"/></value></option></attributes></scheme>"#);
        let chain = SchemeChain {
            layers: vec![&child, &parent],
        };
        let text = chain.attribute(TEXT).unwrap();
        assert_eq!(text.background.as_deref(), Some("222222"));
        assert_eq!(text.background_option, BackgroundOption::Unused);
        // Without a TEXT of its own the child shows the parent's result.
        let bare = one(r#"<scheme name="c" parent_scheme="p"/>"#);
        let chain = SchemeChain {
            layers: vec![&bare, &parent],
        };
        let text = chain.attribute(TEXT).unwrap();
        assert_eq!(text.background.as_deref(), Some("111111"));
        assert_eq!(text.background_option, BackgroundOption::Created);
    }

    #[test]
    fn several_schemes_in_one_document() {
        let doc = r#"<component name="X"><scheme name="One"/><scheme name="Two" parent_scheme="One"/></component>"#;
        let all = Scheme::parse(doc, "x", "x").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].parent.as_deref(), Some("One"));
    }

    #[test]
    fn documents_without_a_scheme_are_rejected() {
        assert!(Scheme::parse("<idea-plugin/>", "x", "x").is_err());
        assert!(Scheme::parse("not xml", "x", "x").is_err());
    }
}
