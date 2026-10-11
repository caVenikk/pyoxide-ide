//! A minimal XML tree: element names, attributes and children. Text content
//! is not needed by colour scheme files and is dropped.

use quick_xml::events::{BytesStart, Event};

/// Deepest nesting accepted; scheme files are about five levels deep.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
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

    /// All descendants (and `self`) with the given name, in document order.
    pub fn descendants_named<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        if self.name == name {
            out.push(self);
            // A scheme does not nest another scheme.
            return;
        }
        for child in &self.children {
            child.descendants_named(name, out);
        }
    }
}

fn element(start: &BytesStart<'_>) -> Result<Element, String> {
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
        children: Vec::new(),
    })
}

/// Parses a whole document and returns its root element.
pub(crate) fn parse(text: &str) -> Result<Element, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("{e} (at byte {})", reader.error_position()))?;
        match event {
            Event::Start(s) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(format!("elements are nested deeper than {MAX_DEPTH}"));
                }
                stack.push(element(&s)?);
            }
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
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("unexpected end of file".into());
    }
    root.ok_or_else(|| "no root element".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_and_attributes() {
        let doc =
            "<?xml version=\"1.0\"?>\n<!-- c --><a x=\"1 &amp; 2\"><b/><c y=\"z\"><d/></c></a>";
        let root = parse(doc).unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.attr("x"), Some("1 & 2"));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.child("c").unwrap().attr("y"), Some("z"));
        assert!(root.child("c").unwrap().child("d").is_some());
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn collects_descendants_without_nesting() {
        let root = parse("<component><scheme n=\"1\"/><x><scheme n=\"2\"><scheme n=\"3\"/></scheme></x></component>").unwrap();
        let mut found = Vec::new();
        root.descendants_named("scheme", &mut found);
        let names: Vec<_> = found.iter().map(|e| e.attr("n").unwrap()).collect();
        assert_eq!(names, ["1", "2"]);
    }

    #[test]
    fn absurd_nesting_is_refused() {
        let deep = "<a>".repeat(10_000);
        assert!(parse(&deep).unwrap_err().contains("nested"));
        let fine = format!("{}{}", "<a>".repeat(60), "</a>".repeat(60));
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn byte_order_mark_is_ignored() {
        assert_eq!(parse("\u{feff}<a/>").unwrap().name, "a");
    }
}
