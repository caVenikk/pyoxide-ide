//! A small in-memory XML tree, enough for IDE settings files.

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Trimmed text of a child element; `None` when absent or empty.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }
}

fn element(start: &quick_xml::events::BytesStart<'_>) -> Result<Element, String> {
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?;
        attrs.push((attr.key.as_ref().to_owned(), value.into_owned()));
    }
    Ok(Element {
        name: start.name().as_ref().to_owned(),
        attrs,
        ..Element::default()
    })
}

/// Parse a whole document; returns the root element.
pub(crate) fn parse(text: &str) -> Result<Element, String> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("{e} (at byte {})", reader.error_position()))?;
        match event {
            Event::Start(s) => stack.push(element(&s)?),
            Event::Empty(s) => {
                let e = element(&s)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(e),
                    None => root = Some(e),
                }
            }
            Event::End(_) => {
                let done = stack.pop().ok_or("unexpected closing tag")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => root = Some(done),
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref().map_err(|e| e.to_string())? {
                    Some(c) => c.to_string(),
                    None => resolve_predefined_entity(&r)
                        .ok_or_else(|| format!("unknown entity &{};", &*r))?
                        .to_owned(),
                };
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&resolved);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("unexpected end of file".into());
    }
    root.ok_or_else(|| "no root element".into())
}

/// Name of the root element, reading only as far as needed.
pub(crate) fn root_name(text: &str) -> Result<Option<String>, String> {
    let mut reader = quick_xml::Reader::from_str(text);
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(s) | Event::Empty(s) => return Ok(Some(s.name().as_ref().to_owned())),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tree_text_and_entities() {
        let doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <a x="1 &amp; 2"><b>t&lt;x&#33;</b><c/><![CDATA[raw <c>]]></a>
</project>"#;
        let root = parse(doc).unwrap();
        assert_eq!(root.name, "project");
        assert_eq!(root.attr("version"), Some("4"));
        let a = root.child("a").unwrap();
        assert_eq!(a.attr("x"), Some("1 & 2"));
        assert_eq!(a.child_text("b"), Some("t<x!"));
        assert_eq!(a.child_text("c"), None);
        assert!(a.text.contains("raw <c>"));
        assert_eq!(root_name(doc).unwrap().as_deref(), Some("project"));
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("").is_err());
        assert!(parse("<a>&bogus;</a>").is_err());
    }
}
