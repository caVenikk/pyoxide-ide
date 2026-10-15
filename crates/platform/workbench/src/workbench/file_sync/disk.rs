//! What the application knows of an open file on disk, and the work on
//! the disk that runs off the UI thread: looking at an open file, and
//! writing one only while it is still what the application last read or
//! wrote (as the reference refreshes a file before saving it and leaves a
//! file that changed meanwhile to the conflict dialog).

use std::path::{Path, PathBuf};

use project::Stamp;
use text::{Decoded, Encoding, LineEnding, ReloadPlan, Rope};

/// A file's content as the application last read or wrote it.
#[derive(Debug, Clone)]
pub(crate) struct Known {
    text: Rope,
    line_ending: LineEnding,
    encoding: Encoding,
}

impl Known {
    pub fn of(buffer: &text::Buffer) -> Self {
        Self {
            text: buffer.rope().clone(),
            line_ending: buffer.line_ending(),
            encoding: buffer.encoding(),
        }
    }

    pub fn decoded(decoded: &Decoded) -> Self {
        Self {
            text: Rope::from_str(&decoded.text),
            line_ending: decoded.line_ending,
            encoding: decoded.encoding,
        }
    }

    pub fn matches(&self, decoded: &Decoded) -> bool {
        self.line_ending == decoded.line_ending
            && self.encoding == decoded.encoding
            && self.text == decoded.text.as_str()
    }
}

/// A change on disk found while the editor had changes of its own.
#[derive(Debug, Clone)]
pub(crate) struct Conflict {
    pub disk: Known,
    pub stamp: Stamp,
}

/// An open file to look at.
pub(crate) struct Check {
    pub path: PathBuf,
    pub epoch: u64,
    /// Skip reading while the file still has this stamp.
    pub stamp: Option<Stamp>,
    pub known: Option<Known>,
    /// The editor's text and state when the check started.
    pub text: Rope,
    pub version: u64,
    pub line_ending: LineEnding,
    pub encoding: Encoding,
    pub read_only: bool,
}

/// What a check found.
pub(crate) enum Checked {
    /// The file is what the application knows (`disk` is set when that
    /// was not known before, or when it is the editor's text).
    Unchanged { stamp: Stamp, disk: Option<Known> },
    /// The file differs from what the application knows.
    Changed {
        disk: Known,
        read_only: bool,
        stamp: Stamp,
        plan: ReloadPlan,
    },
    /// The file is gone.
    Missing,
    /// Not readable as text (left alone).
    Unavailable,
}

impl Check {
    /// Looks at the file (blocking).
    pub fn run(self) -> (PathBuf, u64, u64, Checked) {
        let checked = match self.path.try_exists() {
            Ok(false) => Checked::Missing,
            Err(_) => Checked::Unavailable,
            Ok(true) => self.look(),
        };
        (self.path, self.epoch, self.version, checked)
    }

    fn look(&self) -> Checked {
        let Some(stamp) = Stamp::of(&self.path) else {
            return Checked::Unavailable;
        };
        if Some(stamp) == self.stamp {
            return Checked::Unchanged { stamp, disk: None };
        }
        let Ok(loaded) = project::read_document(&self.path) else {
            return Checked::Unavailable;
        };
        let decoded = &loaded.decoded;
        if self.known.as_ref().is_some_and(|k| k.matches(decoded)) {
            return Checked::Unchanged {
                stamp: loaded.stamp,
                disk: None,
            };
        }
        let read_only = loaded.read_only || decoded.lossy;
        let plan = ReloadPlan::new(&self.text, self.version, decoded);
        let as_editor = !plan.changes_text()
            && decoded.line_ending == self.line_ending
            && decoded.encoding == self.encoding
            && read_only == self.read_only;
        if as_editor {
            return Checked::Unchanged {
                stamp: loaded.stamp,
                disk: Some(Known::decoded(decoded)),
            };
        }
        Checked::Changed {
            disk: Known::decoded(decoded),
            read_only,
            stamp: loaded.stamp,
            plan,
        }
    }
}

/// What a write expects of the file it replaces.
pub(crate) struct SaveGuard {
    /// The file's stamp when the application last read or wrote it.
    stamp: Option<Stamp>,
    /// Contents the file may have (the last known one, and the text of a
    /// write still in flight); empty: write without looking.
    expected: Vec<Known>,
    /// The text being written.
    text: Known,
}

/// How a guarded write ended.
pub(crate) enum Saved {
    Written {
        stamp: Option<Stamp>,
        text: Known,
    },
    /// The file changed on disk since: not written.
    Conflict(Conflict),
    /// The file is gone: not written.
    Missing,
}

impl SaveGuard {
    pub fn new(stamp: Option<Stamp>, expected: Vec<Known>, text: Known) -> Self {
        Self {
            stamp,
            expected,
            text,
        }
    }

    /// Writes `bytes` unless the file changed or went away since the
    /// application last read or wrote it (blocking).
    pub fn write(self, path: &Path, bytes: &[u8]) -> std::io::Result<Saved> {
        if !self.expected.is_empty() {
            if matches!(path.try_exists(), Ok(false)) {
                return Ok(Saved::Missing);
            }
            let now = Stamp::of(path);
            if now.is_some()
                && now != self.stamp
                && let Ok(loaded) = project::read_document(path)
                && !self.expected.iter().any(|k| k.matches(&loaded.decoded))
            {
                return Ok(Saved::Conflict(Conflict {
                    disk: Known::decoded(&loaded.decoded),
                    stamp: loaded.stamp,
                }));
            }
        }
        project::write_document(path, bytes)?;
        Ok(Saved::Written {
            stamp: Stamp::of(path),
            text: self.text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(text: &str) -> Known {
        Known::of(&text::Buffer::new(text))
    }

    #[test]
    fn a_guarded_write_leaves_a_file_changed_or_removed_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        std::fs::write(&path, "a = 1\n").unwrap();
        let stamp = Stamp::of(&path);
        let guard = |expected| SaveGuard::new(stamp, expected, known("a = 2\n"));

        // Unchanged since: written.
        let saved = guard(vec![known("a = 1\n")])
            .write(&path, b"a = 2\n")
            .unwrap();
        assert!(matches!(saved, Saved::Written { stamp: Some(_), .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2\n");

        // Changed by someone else: not written.
        std::fs::write(&path, "outside = 1\n").unwrap();
        let saved = guard(vec![known("a = 2\n")])
            .write(&path, b"a = 3\n")
            .unwrap();
        let Saved::Conflict(conflict) = saved else {
            panic!("a conflict")
        };
        assert!(
            conflict
                .disk
                .matches(&text::decode(b"outside = 1\n").unwrap())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "outside = 1\n");

        // Touched without a change, or written by a save still in flight:
        // written.
        let expected = vec![known("a = 2\n"), known("outside = 1\n")];
        let saved = guard(expected).write(&path, b"a = 4\n").unwrap();
        assert!(matches!(saved, Saved::Written { .. }));

        // Gone: not written (not made again).
        std::fs::remove_file(&path).unwrap();
        let saved = guard(vec![known("a = 4\n")])
            .write(&path, b"a = 5\n")
            .unwrap();
        assert!(matches!(saved, Saved::Missing));
        assert!(!path.exists());
        // Without expectations (an explicit save of a deleted file): made.
        let saved = guard(Vec::new()).write(&path, b"a = 5\n").unwrap();
        assert!(matches!(saved, Saved::Written { .. }));
        assert!(path.exists());
    }
}
