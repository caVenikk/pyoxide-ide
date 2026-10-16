//! Breakpoints as the reference keeps them (`XBreakpointManagerImpl`,
//! `BreakpointState`, `LineBreakpointState`): line breakpoints of files
//! and exception breakpoints, each enabled or not, with a suspend policy
//! and a condition; line breakpoints may log instead of suspending and may
//! be temporary (removed when hit). Lines count from 1, as shown.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A breakpoint's identity while the project is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BreakpointId(pub u64);

/// What a hit suspends (`SuspendPolicy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SuspendPolicy {
    #[default]
    All,
    Thread,
    /// Nothing: the breakpoint only logs.
    None,
}

impl SuspendPolicy {
    /// The stored name (`all`, `thread`, `none`).
    pub fn as_str(self) -> &'static str {
        match self {
            SuspendPolicy::All => "all",
            SuspendPolicy::Thread => "thread",
            SuspendPolicy::None => "none",
        }
    }

    /// From the stored name or the reference's (`ALL`, `THREAD`, `NONE`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "all" => Some(SuspendPolicy::All),
            "thread" => Some(SuspendPolicy::Thread),
            "none" => Some(SuspendPolicy::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBreakpoint {
    pub id: BreakpointId,
    /// An absolute path.
    pub file: PathBuf,
    /// From 1.
    pub line: u32,
    pub enabled: bool,
    pub suspend: SuspendPolicy,
    pub condition: Option<String>,
    /// Logs the "Breakpoint hit" message.
    pub log_message: bool,
    /// Logs the stack trace.
    pub log_stack: bool,
    /// Evaluates this expression and logs the value.
    pub log_expression: Option<String>,
    /// Removed once hit.
    pub temporary: bool,
}

impl LineBreakpoint {
    /// An enabled breakpoint with the default policy on a line.
    pub fn new(file: impl Into<PathBuf>, line: u32) -> Self {
        Self {
            id: BreakpointId(0),
            file: file.into(),
            line,
            enabled: true,
            suspend: SuspendPolicy::All,
            condition: None,
            log_message: false,
            log_stack: false,
            log_expression: None,
            temporary: false,
        }
    }

    /// Suspends nothing but logs (the reference's non-suspending logging
    /// breakpoint).
    pub fn logs_only(&self) -> bool {
        self.suspend == SuspendPolicy::None
            && (self.log_message || self.log_stack || self.log_expression.is_some())
    }
}

/// An exception breakpoint of a product's type (`python-exception`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionBreakpoint {
    pub id: BreakpointId,
    /// The type, as the reference names it.
    pub kind: String,
    /// The exception class; the product names its "any exception" class.
    pub class: String,
    /// The type's default breakpoint ("Any exception"): kept, never
    /// removed.
    pub default: bool,
    pub enabled: bool,
    pub suspend: SuspendPolicy,
    pub condition: Option<String>,
    /// The product's options (`notifyOnTerminate` = `true`), as the
    /// reference stores them.
    pub properties: BTreeMap<String, String>,
}

impl ExceptionBreakpoint {
    pub fn new(kind: impl Into<String>, class: impl Into<String>) -> Self {
        Self {
            id: BreakpointId(0),
            kind: kind.into(),
            class: class.into(),
            default: false,
            enabled: true,
            suspend: SuspendPolicy::All,
            condition: None,
            properties: BTreeMap::new(),
        }
    }

    /// A property is `true`.
    pub fn flag(&self, name: &str) -> bool {
        self.properties
            .get(name)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }
}

/// What [`Breakpoints::toggle`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggled {
    Added(BreakpointId),
    Removed(BreakpointId),
}

/// A project's breakpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakpoints {
    line: Vec<LineBreakpoint>,
    exception: Vec<ExceptionBreakpoint>,
    next_id: u64,
}

impl Breakpoints {
    pub fn line(&self) -> &[LineBreakpoint] {
        &self.line
    }

    pub fn exception(&self) -> &[ExceptionBreakpoint] {
        &self.exception
    }

    fn take_id(&mut self) -> BreakpointId {
        self.next_id += 1;
        BreakpointId(self.next_id)
    }

    /// The breakpoint on a line of a file.
    pub fn at(&self, file: &Path, line: u32) -> Option<&LineBreakpoint> {
        self.line.iter().find(|b| b.file == file && b.line == line)
    }

    /// The line breakpoints of a file, by line.
    pub fn in_file(&self, file: &Path) -> Vec<&LineBreakpoint> {
        let mut found: Vec<&LineBreakpoint> = self.line.iter().filter(|b| b.file == file).collect();
        found.sort_by_key(|b| b.line);
        found
    }

    /// The files with line breakpoints.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.line.iter().map(|b| b.file.clone()).collect();
        files.sort();
        files.dedup();
        files
    }

    pub fn get(&self, id: BreakpointId) -> Option<&LineBreakpoint> {
        self.line.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: BreakpointId) -> Option<&mut LineBreakpoint> {
        self.line.iter_mut().find(|b| b.id == id)
    }

    pub fn exception_mut(&mut self, id: BreakpointId) -> Option<&mut ExceptionBreakpoint> {
        self.exception.iter_mut().find(|b| b.id == id)
    }

    /// Adds a line breakpoint (its id is assigned); one on the same line
    /// is replaced.
    pub fn add_line(&mut self, mut breakpoint: LineBreakpoint) -> BreakpointId {
        self.line
            .retain(|b| !(b.file == breakpoint.file && b.line == breakpoint.line));
        breakpoint.id = self.take_id();
        let id = breakpoint.id;
        self.line.push(breakpoint);
        id
    }

    /// Toggle Line Breakpoint: removes the breakpoint of the line, or adds
    /// one.
    pub fn toggle(&mut self, file: &Path, line: u32, temporary: bool) -> Toggled {
        if let Some(existing) = self.at(file, line) {
            let id = existing.id;
            self.remove(id);
            return Toggled::Removed(id);
        }
        let mut breakpoint = LineBreakpoint::new(file, line);
        breakpoint.temporary = temporary;
        Toggled::Added(self.add_line(breakpoint))
    }

    /// Removes a line breakpoint, or an exception breakpoint that is not a
    /// default one.
    pub fn remove(&mut self, id: BreakpointId) -> bool {
        let before = self.line.len() + self.exception.len();
        self.line.retain(|b| b.id != id);
        self.exception.retain(|b| b.id != id || b.default);
        before != self.line.len() + self.exception.len()
    }

    /// Adds an exception breakpoint (its id is assigned); one of the same
    /// type and class is replaced, keeping a default one default.
    pub fn add_exception(&mut self, mut breakpoint: ExceptionBreakpoint) -> BreakpointId {
        if let Some(i) = self
            .exception
            .iter()
            .position(|b| b.kind == breakpoint.kind && b.class == breakpoint.class)
        {
            breakpoint.default |= self.exception[i].default;
            self.exception.remove(i);
        }
        breakpoint.id = self.take_id();
        let id = breakpoint.id;
        self.exception.push(breakpoint);
        id
    }

    /// Lines `first..first + old_count` of a file were replaced by
    /// `new_count` lines: breakpoints below move, breakpoints on lines that
    /// are gone are removed. Returns whether anything changed.
    pub fn lines_changed(
        &mut self,
        file: &Path,
        first: u32,
        old_count: u32,
        new_count: u32,
    ) -> bool {
        let end = first + old_count;
        let before = self.line.clone();
        self.line.retain(|b| {
            !(b.file == file && b.line >= first + new_count && b.line < end && b.line != first)
        });
        for b in self.line.iter_mut().filter(|b| b.file == file) {
            if b.line >= end {
                b.line = b.line + new_count - old_count;
            }
        }
        // Two on one line: the first one stays.
        let mut seen = std::collections::HashSet::new();
        self.line.retain(|b| seen.insert((b.file.clone(), b.line)));
        before != self.line
    }

    /// A file was renamed or moved: its breakpoints follow.
    pub fn file_moved(&mut self, from: &Path, to: &Path) -> bool {
        let mut changed = false;
        for b in &mut self.line {
            if let Ok(rest) = b.file.strip_prefix(from) {
                b.file = if rest.as_os_str().is_empty() {
                    to.to_owned()
                } else {
                    to.join(rest)
                };
                changed = true;
            }
        }
        changed
    }
}

/// Watch expressions by key: the reference keeps them per run
/// configuration type (`XDebugSessionData`, keyed by the configuration
/// type's id), so every configuration of a type shares them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watches {
    by_key: BTreeMap<String, Vec<String>>,
}

impl Watches {
    pub fn get(&self, key: &str) -> &[String] {
        self.by_key.get(key).map_or(&[], Vec::as_slice)
    }

    pub fn keys(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.by_key.iter()
    }

    /// Adds a watch at `index` (the end with `None`); an empty expression
    /// is not added.
    pub fn add(&mut self, key: &str, expression: &str, index: Option<usize>) -> bool {
        let expression = expression.trim();
        if expression.is_empty() {
            return false;
        }
        let list = self.by_key.entry(key.to_owned()).or_default();
        let at = index.unwrap_or(list.len()).min(list.len());
        list.insert(at, expression.to_owned());
        true
    }

    pub fn remove(&mut self, key: &str, index: usize) -> Option<String> {
        let list = self.by_key.get_mut(key)?;
        if index >= list.len() {
            return None;
        }
        let removed = list.remove(index);
        if list.is_empty() {
            self.by_key.remove(key);
        }
        Some(removed)
    }

    /// Replaces a watch; an empty expression removes it.
    pub fn edit(&mut self, key: &str, index: usize, expression: &str) -> bool {
        let expression = expression.trim();
        if expression.is_empty() {
            return self.remove(key, index).is_some();
        }
        match self.by_key.get_mut(key).and_then(|l| l.get_mut(index)) {
            Some(slot) => {
                *slot = expression.to_owned();
                true
            }
            None => false,
        }
    }

    pub fn set(&mut self, key: &str, expressions: Vec<String>) {
        if expressions.is_empty() {
            self.by_key.remove(key);
        } else {
            self.by_key.insert(key.to_owned(), expressions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_adds_and_removes() {
        let mut b = Breakpoints::default();
        let file = Path::new("/p/main.py");
        let Toggled::Added(id) = b.toggle(file, 3, false) else {
            panic!()
        };
        assert_eq!(b.at(file, 3).unwrap().id, id);
        assert!(b.at(file, 3).unwrap().enabled);
        assert_eq!(b.toggle(file, 3, false), Toggled::Removed(id));
        assert!(b.line().is_empty());
        let Toggled::Added(other) = b.toggle(file, 3, true) else {
            panic!()
        };
        assert_ne!(other, id);
        assert!(b.get(other).unwrap().temporary);
    }

    #[test]
    fn edits_move_and_remove_breakpoints() {
        let mut b = Breakpoints::default();
        let file = Path::new("/p/main.py");
        for line in [2, 5, 6, 9] {
            b.toggle(file, line, false);
        }
        b.toggle(Path::new("/p/other.py"), 9, false);
        // Two lines inserted after line 3.
        assert!(b.lines_changed(file, 4, 0, 2));
        let lines: Vec<u32> = b.in_file(file).iter().map(|b| b.line).collect();
        assert_eq!(lines, [2, 7, 8, 11]);
        // Lines 7 and 8 joined into one: the breakpoint of line 8 goes.
        assert!(b.lines_changed(file, 7, 2, 1));
        let lines: Vec<u32> = b.in_file(file).iter().map(|b| b.line).collect();
        assert_eq!(lines, [2, 7, 10]);
        assert_eq!(b.in_file(Path::new("/p/other.py"))[0].line, 9);
        assert!(!b.lines_changed(file, 12, 1, 1));
        assert!(b.file_moved(Path::new("/p"), Path::new("/q")));
        assert_eq!(
            b.files(),
            [PathBuf::from("/q/main.py"), "/q/other.py".into()]
        );
    }

    #[test]
    fn default_exception_breakpoints_stay() {
        let mut b = Breakpoints::default();
        let mut any = ExceptionBreakpoint::new("python-exception", "BaseException");
        any.default = true;
        let id = b.add_exception(any);
        assert!(!b.remove(id));
        let mut again = ExceptionBreakpoint::new("python-exception", "BaseException");
        again.enabled = false;
        let id = b.add_exception(again);
        assert_eq!(b.exception().len(), 1);
        assert!(b.exception()[0].default && !b.exception()[0].enabled);
        let value_error =
            b.add_exception(ExceptionBreakpoint::new("python-exception", "ValueError"));
        assert!(b.remove(value_error));
        assert_eq!(b.exception()[0].id, id);
    }

    #[test]
    fn watches_per_key() {
        let mut w = Watches::default();
        assert!(w.add("python", "x", None));
        assert!(w.add("python", " y + 1 ", None));
        assert!(w.add("python", "z", Some(0)));
        assert!(!w.add("python", "  ", None));
        assert_eq!(w.get("python"), ["z", "x", "y + 1"]);
        assert!(w.get("shell-script").is_empty());
        assert!(w.edit("python", 1, "x.y"));
        assert_eq!(w.remove("python", 0).as_deref(), Some("z"));
        assert!(w.edit("python", 0, ""));
        assert_eq!(w.get("python"), ["y + 1"]);
        assert_eq!(w.remove("python", 5), None);
    }
}
