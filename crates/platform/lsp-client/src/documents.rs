//! The open documents: their language, version and text, and the recent
//! history of their changes, so answers computed for an older text
//! (diagnostics) can be moved through the edits made since.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use code_intel::{AppliedEdit, DocumentId};
use text::Rope;

/// The number of past versions kept per document: an answer for a text
/// older than that is dropped.
const HISTORY: usize = 64;

/// A document as of one version.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub language: String,
    pub version: u64,
    pub text: Rope,
}

#[derive(Debug)]
struct Doc {
    language: String,
    version: u64,
    text: Rope,
    /// Oldest first.
    history: VecDeque<Step>,
}

/// From one version to the next.
#[derive(Debug)]
struct Step {
    /// The version before the step and its text.
    version: u64,
    text: Rope,
    /// The edits of the step; `None` when the text was replaced as a whole.
    edits: Option<Vec<AppliedEdit>>,
}

#[derive(Debug, Default)]
pub(crate) struct Documents {
    docs: Mutex<HashMap<DocumentId, Doc>>,
}

/// Moves char offsets of an older text through the edits made since.
#[derive(Debug, Clone, Default)]
pub(crate) struct Mapper {
    edits: Vec<AppliedEdit>,
}

/// Where an offset inside a replaced range goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bias {
    /// To the start of the replacement (a range's start).
    Start,
    /// To the end of the replacement (a range's end).
    End,
}

impl Mapper {
    #[cfg(test)]
    pub fn is_identity(&self) -> bool {
        self.edits.is_empty()
    }

    /// Moves an offset as a range marker of the editor moves: text
    /// inserted at a range's start goes before it, text inserted at its
    /// end stays outside it; a range end inside replaced text goes to the
    /// replacement's end, a start to its start.
    pub fn map(&self, mut offset: usize, bias: Bias) -> usize {
        for edit in &self.edits {
            let insertion = edit.start == edit.old_end;
            offset = if offset < edit.start {
                offset
            } else if offset == edit.start && insertion {
                match bias {
                    Bias::Start => edit.new_end,
                    Bias::End => offset,
                }
            } else if offset == edit.start {
                offset
            } else if offset >= edit.old_end {
                offset - edit.old_end + edit.new_end
            } else {
                match bias {
                    Bias::Start => edit.start,
                    Bias::End => edit.new_end,
                }
            };
        }
        offset
    }
}

impl Documents {
    fn lock(&self) -> MutexGuard<'_, HashMap<DocumentId, Doc>> {
        self.docs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn open(&self, doc: &DocumentId, language: &str, version: u64, text: &Rope) {
        self.lock().insert(
            doc.clone(),
            Doc {
                language: language.to_owned(),
                version,
                text: text.clone(),
                history: VecDeque::new(),
            },
        );
    }

    /// Records a change; the version before it, `None` for a document that
    /// is not open.
    pub fn change(
        &self,
        doc: &DocumentId,
        version: u64,
        edits: Option<&[AppliedEdit]>,
        text: &Rope,
    ) -> Option<u64> {
        let mut docs = self.lock();
        let entry = docs.get_mut(doc)?;
        let before = entry.version;
        let old = std::mem::replace(&mut entry.text, text.clone());
        entry.history.push_back(Step {
            version: before,
            text: old,
            edits: edits.map(<[AppliedEdit]>::to_vec),
        });
        if entry.history.len() > HISTORY {
            entry.history.pop_front();
        }
        entry.version = version;
        Some(before)
    }

    pub fn close(&self, doc: &DocumentId) -> bool {
        self.lock().remove(doc).is_some()
    }

    pub fn get(&self, doc: &DocumentId) -> Option<Snapshot> {
        self.lock().get(doc).map(|d| Snapshot {
            language: d.language.clone(),
            version: d.version,
            text: d.text.clone(),
        })
    }

    pub fn is_open(&self, doc: &DocumentId) -> bool {
        self.lock().contains_key(doc)
    }

    /// The open documents.
    pub fn all(&self) -> Vec<(DocumentId, Snapshot)> {
        self.lock()
            .iter()
            .map(|(id, d)| {
                (
                    id.clone(),
                    Snapshot {
                        language: d.language.clone(),
                        version: d.version,
                        text: d.text.clone(),
                    },
                )
            })
            .collect()
    }

    /// The text of an earlier (or the current) version, while it is kept.
    pub fn text_at(&self, doc: &DocumentId, version: u64) -> Option<Rope> {
        let docs = self.lock();
        let entry = docs.get(doc)?;
        if entry.version == version {
            return Some(entry.text.clone());
        }
        entry
            .history
            .iter()
            .find(|s| s.version == version)
            .map(|s| s.text.clone())
    }

    /// The current version and how to move offsets of `version` to it;
    /// `None` when that version is no longer kept or the text was replaced
    /// as a whole since.
    pub fn mapper(&self, doc: &DocumentId, version: u64) -> Option<(u64, Mapper)> {
        let docs = self.lock();
        let entry = docs.get(doc)?;
        if entry.version == version {
            return Some((version, Mapper::default()));
        }
        let from = entry.history.iter().position(|s| s.version == version)?;
        let mut edits = Vec::new();
        for step in entry.history.iter().skip(from) {
            edits.extend(step.edits.as_ref()?.iter().cloned());
        }
        Some((entry.version, Mapper { edits }))
    }
}

#[cfg(test)]
mod tests {
    use text::{Buffer, CaretPlacement, Edit, EditKind, SelectionSet};

    use super::*;

    fn edit(buffer: &mut Buffer, edits: Vec<Edit>) -> Vec<AppliedEdit> {
        let mut selections = SelectionSet::caret(0);
        buffer.transact(
            &mut selections,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        )
    }

    #[test]
    fn offsets_move_through_later_edits() {
        let docs = Documents::default();
        let doc = DocumentId::new("/p/a.py");
        let mut buffer = Buffer::new("import os\nx = 1\n");
        docs.open(&doc, "python", 1, buffer.rope());
        // `os` is 7..9 in version 1.
        let applied = edit(
            &mut buffer,
            vec![Edit::insert(0, "# hi\n"), Edit::new(10..11, "y")],
        );
        assert_eq!(docs.change(&doc, 2, Some(&applied), buffer.rope()), Some(1));
        let applied = edit(&mut buffer, vec![Edit::new(12..14, "sys")]);
        docs.change(&doc, 3, Some(&applied), buffer.rope());
        assert_eq!(buffer.text(), "# hi\nimport sys\ny = 1\n");

        let (current, mapper) = docs.mapper(&doc, 1).unwrap();
        assert_eq!(current, 3);
        // The replaced name: start and end at the replacement's ends.
        assert_eq!(mapper.map(7, Bias::Start), 12);
        assert_eq!(mapper.map(9, Bias::End), 15);
        // `x` became `y` at the same place, after the inserted line.
        assert_eq!(mapper.map(10, Bias::Start), 16);
        assert_eq!(
            docs.text_at(&doc, 1).unwrap().to_string(),
            "import os\nx = 1\n"
        );
        assert!(docs.mapper(&doc, 3).unwrap().1.is_identity());

        // A reload breaks the chain.
        docs.change(&doc, 4, None, &Rope::from_str("new\n"));
        assert!(docs.mapper(&doc, 3).is_none());
        assert!(docs.mapper(&doc, 4).is_some());
        assert_eq!(docs.get(&doc).unwrap().version, 4);
        assert!(docs.close(&doc));
        assert!(docs.get(&doc).is_none());
        assert_eq!(docs.change(&doc, 5, None, &Rope::new()), None);
    }

    #[test]
    fn history_is_bounded() {
        let docs = Documents::default();
        let doc = DocumentId::new("/p/a.py");
        let mut buffer = Buffer::new("");
        docs.open(&doc, "python", 0, buffer.rope());
        for version in 1..=(HISTORY as u64 + 10) {
            let applied = edit(&mut buffer, vec![Edit::insert(0, "x")]);
            docs.change(&doc, version, Some(&applied), buffer.rope());
        }
        assert!(docs.mapper(&doc, 0).is_none());
        let (_, mapper) = docs.mapper(&doc, 20).unwrap();
        // Text inserted at a start goes before it, at an end after it.
        assert_eq!(mapper.map(0, Bias::Start), HISTORY + 10 - 20);
        assert_eq!(mapper.map(0, Bias::End), 0);
    }
}
