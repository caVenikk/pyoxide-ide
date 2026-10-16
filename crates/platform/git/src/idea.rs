//! The reference IDE's version control mapping of a project
//! (`.idea/vcs.xml`, component `VcsDirectoryMappings`), read and never
//! written.

use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// The version control a project is mapped to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapping {
    /// Mapped to no version control: Git stays off.
    None,
    /// Mapped to a version control by its name (`Git`).
    Vcs(String),
}

/// The mapping of the project folder in `<root>/.idea/vcs.xml`; `None`
/// when the file or a mapping of the project folder is missing.
pub fn project_mapping(root: &Path) -> Option<Mapping> {
    let text = std::fs::read_to_string(root.join(".idea").join("vcs.xml")).ok()?;
    parse(&text)
}

fn parse(text: &str) -> Option<Mapping> {
    let mut reader = Reader::from_str(text);
    let mut in_mappings = false;
    let mut found = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                "component" => {
                    in_mappings = attr(&e, "name").as_deref() == Some("VcsDirectoryMappings");
                }
                "mapping" if in_mappings => {
                    let directory = attr(&e, "directory").unwrap_or_default();
                    // The project folder: `$PROJECT_DIR$`, or the empty
                    // directory of the "<Project>" mapping.
                    if directory.is_empty() || directory == "$PROJECT_DIR$" {
                        let vcs = attr(&e, "vcs").unwrap_or_default();
                        found = Some(if vcs.is_empty() {
                            Mapping::None
                        } else {
                            Mapping::Vcs(vcs)
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.name().as_ref() == "component" => in_mappings = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    found
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == name)
            .then(|| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_folders_mapping_is_read() {
        let none = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="VcsDirectoryMappings">
    <mapping directory="" vcs="" />
  </component>
</project>"#;
        assert_eq!(parse(none), Some(Mapping::None));
        let git = r#"<project version="4"><component name="VcsDirectoryMappings">
<mapping directory="$PROJECT_DIR$/lib" vcs="" />
<mapping directory="$PROJECT_DIR$" vcs="Git" /></component></project>"#;
        assert_eq!(parse(git), Some(Mapping::Vcs("Git".into())));
        let other = r#"<project><component name="Other"><mapping directory="" vcs="" /></component></project>"#;
        assert_eq!(parse(other), None);
    }
}
