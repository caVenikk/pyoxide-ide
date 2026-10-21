//! What the reference IDE keeps of a project's version control, read and
//! never written: the mapping (`.idea/vcs.xml`, component
//! `VcsDirectoryMappings`) and the commit messages (`.idea/workspace.xml`:
//! the default changelist's description, the unsent message, and
//! `VcsManagerConfiguration`'s recent messages).

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

/// The commit messages of the reference's personal project state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitMessages {
    /// The unsent message (`ChangeListManager`: the default changelist's
    /// `comment`, where `ChangesViewCommitMessagePolicy` keeps it).
    pub draft: Option<String>,
    /// The recent messages, the last one first (`VcsManagerConfiguration`'s
    /// `MESSAGE` values, stored the last one last).
    pub recent: Vec<String>,
}

/// The commit messages in `<root>/.idea/workspace.xml`; `None` when the
/// file cannot be read.
pub fn commit_messages(root: &Path) -> Option<CommitMessages> {
    let text = std::fs::read_to_string(root.join(".idea").join("workspace.xml")).ok()?;
    Some(parse_messages(&text))
}

fn parse_messages(text: &str) -> CommitMessages {
    let mut reader = Reader::from_str(text);
    let mut component = String::new();
    let mut out = CommitMessages::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                "component" => component = attr(&e, "name").unwrap_or_default(),
                "list"
                    if component == "ChangeListManager"
                        && attr(&e, "default").as_deref() == Some("true") =>
                {
                    out.draft = attr(&e, "comment").filter(|c| !c.trim().is_empty());
                }
                "MESSAGE" if component == "VcsManagerConfiguration" => {
                    if let Some(m) = attr(&e, "value").filter(|m| !m.trim().is_empty()) {
                        out.recent.retain(|r| *r != m);
                        out.recent.insert(0, m);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.name().as_ref() == "component" => component.clear(),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
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

    #[test]
    fn the_unsent_and_the_recent_messages_are_read() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="ChangeListManager">
    <list id="1" name="Other" comment="not this" />
    <list default="true" id="2" name="Changes" comment="Fix the parser&#10;&#10;Details">
      <change afterPath="$PROJECT_DIR$/a.py" afterDir="false" />
    </list>
  </component>
  <component name="VcsManagerConfiguration">
    <MESSAGE value="first" />
    <MESSAGE value="second &amp; more" />
    <option name="LAST_COMMIT_MESSAGE" value="second &amp; more" />
  </component>
  <component name="Other"><MESSAGE value="no" /></component>
</project>"#;
        assert_eq!(
            parse_messages(text),
            CommitMessages {
                draft: Some("Fix the parser\n\nDetails".into()),
                recent: vec!["second & more".into(), "first".into()],
            }
        );
        let empty = r#"<project><component name="ChangeListManager">
<list default="true" id="2" name="Changes" comment="" /></component></project>"#;
        assert_eq!(parse_messages(empty), CommitMessages::default());
    }
}
