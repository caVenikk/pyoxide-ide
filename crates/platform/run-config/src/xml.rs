//! A small read-only element tree over `quick-xml` for the reference IDE's
//! settings files: names, attributes, children and text, and where each
//! element is in the source, so that an element can be kept byte for byte.

use std::ops::Range;

use quick_xml::events::{BytesStart, Event};

/// An element of a document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The text directly inside the element, entities resolved.
    pub text: String,
    /// The element in the source document, from `<` to its end.
    pub span: Range<usize>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The attribute is `true` (`Boolean.parseBoolean`, case ignored).
    pub fn bool_attr(&self, name: &str) -> bool {
        self.attr(name)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The value of a field the way the reference writes one
    /// (`JDOMExternalizerUtil.writeField`): `<option name="X" value="Y"/>`
    /// directly inside the element. `None` when it is missing.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.children_named("option")
            .find(|o| o.attr("name") == Some(name))
            .map(|o| o.attr("value").unwrap_or(""))
    }

    /// The `<option name="X">` element itself (for nested values).
    pub fn option_element(&self, name: &str) -> Option<&Element> {
        self.children_named("option")
            .find(|o| o.attr("name") == Some(name))
    }

    /// The element as it is written in `source`.
    pub fn source<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.span.clone()).unwrap_or("")
    }
}

/// Parses a document and returns its root element.
pub fn parse(text: &str) -> Result<Element, String> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(e) => {
                let mut element = element(&e)?;
                element.span = start..start;
                stack.push(element);
            }
            Event::Empty(e) => {
                let mut element = element(&e)?;
                element.span = start..reader.buffer_position() as usize;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => root = root.or(Some(element)),
                }
            }
            Event::End(_) => {
                let Some(mut element) = stack.pop() else {
                    return Err("unbalanced end tag".into());
                };
                element.span.end = reader.buffer_position() as usize;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => root = root.or(Some(element)),
                }
            }
            Event::Text(t) => {
                if let Some(e) = stack.last_mut() {
                    e.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(e) = stack.last_mut() {
                    e.text.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                if let Some(e) = stack.last_mut() {
                    let name = r.xml10_content();
                    let entity = format!("&{name};");
                    match quick_xml::escape::unescape(&entity) {
                        Ok(text) => e.text.push_str(&text),
                        Err(_) => e.text.push_str(&entity),
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("unexpected end of the document".into());
    }
    root.ok_or_else(|| "no root element".into())
}

fn element(e: &BytesStart<'_>) -> Result<Element, String> {
    let name = e.name().as_ref().to_owned();
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| e.to_string())?;
        let key = a.key.as_ref().to_owned();
        let value = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(Element {
        name,
        attrs,
        ..Element::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_text_and_spans() {
        let text = "<?xml version=\"1.0\"?>\n<project version=\"4\">\n  <component name=\"A\">{ &quot;k&quot;: 1 }</component>\n  <component name=\"B\">\n    <option name=\"X\" value=\"a &amp; b\" />\n    <option name=\"E\" />\n  </component>\n</project>\n";
        let root = parse(text).unwrap();
        assert_eq!(root.name, "project");
        assert_eq!(root.attr("version"), Some("4"));
        let a = &root.children[0];
        assert_eq!(a.text, "{ \"k\": 1 }");
        let b = root
            .children_named("component")
            .find(|c| c.attr("name") == Some("B"))
            .unwrap();
        assert_eq!(b.field("X"), Some("a & b"));
        assert_eq!(b.field("E"), Some(""));
        assert_eq!(b.field("missing"), None);
        assert!(b.source(text).starts_with("<component name=\"B\">"));
        assert!(b.source(text).ends_with("</component>"));
        assert_eq!(
            b.children[0].source(text),
            "<option name=\"X\" value=\"a &amp; b\" />"
        );
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("").is_err());
    }
}
