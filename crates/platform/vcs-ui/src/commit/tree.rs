//! The changes tree of the Commit tool window as data (the reference's
//! `ChangesListView` with its inclusion model): the "Changes" group with
//! the changes against `HEAD` and the "Unversioned Files" group, flat
//! (each file with its parent folder), check boxes that include files in
//! the commit, expansion and a selection of rows.
//!
//! Inclusion follows the reference's partial commit model: changes are
//! included when the window first gets them and by Commit (⌘K); a change
//! that appears later is included while every other change is;
//! unversioned files are included only by hand.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use vcs::{Change, FileStatus};

/// A group of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// The changes against `HEAD` (the default change list, "Changes").
    Changes,
    /// Files that are not tracked ("Unversioned Files").
    Unversioned,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            // `changes.default.changelist.name`,
            // `changes.nodetitle.unversioned.files`.
            Group::Changes => "Changes",
            Group::Unversioned => "Unversioned Files",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Group::Changes => "changes",
            Group::Unversioned => "unversioned",
        }
    }
}

/// A row: a group or a file of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Row {
    Group(Group),
    File(PathBuf),
}

/// The check box of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inclusion {
    None,
    Some,
    All,
}

impl Inclusion {
    pub fn id(self) -> &'static str {
        match self {
            Inclusion::None => "none",
            Inclusion::Some => "some",
            Inclusion::All => "all",
        }
    }
}

/// "3 files" (`changes.nodetitle.changecount`).
pub fn file_count(n: usize) -> String {
    match n {
        0 => "No files".to_owned(),
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    }
}

/// The tree's data.
#[derive(Debug, Clone)]
pub struct ChangesTree {
    root: PathBuf,
    changes: Vec<Change>,
    unversioned: Vec<PathBuf>,
    included: HashSet<PathBuf>,
    /// Changes were given at least once.
    loaded: bool,
    expanded_changes: bool,
    expanded_unversioned: bool,
    selected: HashSet<Row>,
    /// The row the keyboard moves from.
    lead: Option<Row>,
    /// Where a Shift+click range starts.
    anchor: Option<Row>,
    /// Each listed file's group and place in it.
    index: HashMap<PathBuf, (Group, usize)>,
}

impl Default for ChangesTree {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            changes: Vec::new(),
            unversioned: Vec::new(),
            included: HashSet::new(),
            loaded: false,
            expanded_changes: true,
            // The reference expands the default change list only.
            expanded_unversioned: false,
            selected: HashSet::new(),
            lead: None,
            anchor: None,
            index: HashMap::new(),
        }
    }
}

impl ChangesTree {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn unversioned(&self) -> &[PathBuf] {
        &self.unversioned
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.unversioned.is_empty()
    }

    /// New statuses of the work tree: inclusion and selection stay for
    /// the paths that are still there.
    pub fn set_changes(&mut self, root: &Path, changes: Vec<Change>, unversioned: Vec<PathBuf>) {
        let first = !self.loaded || self.root != root;
        let all_included = !self.changes.is_empty()
            && self.changes.iter().all(|c| self.included.contains(&c.path));
        let old: HashSet<PathBuf> = self.changes.iter().map(|c| c.path.clone()).collect();
        if self.root != root {
            *self = Self::default();
        }
        self.root = root.to_owned();
        self.loaded = true;
        let paths: HashSet<&Path> = changes
            .iter()
            .map(|c| c.path.as_path())
            .chain(unversioned.iter().map(PathBuf::as_path))
            .collect();
        self.included.retain(|p| paths.contains(p.as_path()));
        for c in &changes {
            let new = !old.contains(&c.path);
            if first || (new && all_included) {
                self.included.insert(c.path.clone());
            }
        }
        // A file that moved from Unversioned to Changes (Add to VCS) keeps
        // its check box. Without grouping, files go by name, case ignored,
        // then by path (`ChangesComparator`'s flat order).
        let mut changes = changes;
        changes.sort_by(|a, b| by_name(&a.path, &b.path));
        let mut unversioned = unversioned;
        unversioned.sort_by(|a, b| by_name(a, b));
        self.index = changes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.path.clone(), (Group::Changes, i)))
            .chain(
                unversioned
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.clone(), (Group::Unversioned, i))),
            )
            .collect();
        self.changes = changes;
        self.unversioned = unversioned;
        let rows: HashSet<Row> = self.rows().into_iter().collect();
        self.selected.retain(|r| rows.contains(r));
        if self.lead.as_ref().is_some_and(|l| !rows.contains(l)) {
            self.lead = None;
        }
        if self.anchor.as_ref().is_some_and(|a| !rows.contains(a)) {
            self.anchor = None;
        }
        if self.selected.is_empty()
            && let Some(first) = self.rows().into_iter().next()
        {
            self.select(&first);
        }
    }

    /// The group a file is in.
    pub fn group_of(&self, path: &Path) -> Option<Group> {
        self.index.get(path).map(|(g, _)| *g)
    }

    pub fn change(&self, path: &Path) -> Option<&Change> {
        match self.index.get(path) {
            Some((Group::Changes, i)) => self.changes.get(*i),
            _ => None,
        }
    }

    /// The files of a group.
    pub fn files(&self, group: Group) -> Vec<PathBuf> {
        match group {
            Group::Changes => self.changes.iter().map(|c| c.path.clone()).collect(),
            Group::Unversioned => self.unversioned.clone(),
        }
    }

    pub fn is_expanded(&self, group: Group) -> bool {
        match group {
            Group::Changes => self.expanded_changes,
            Group::Unversioned => self.expanded_unversioned,
        }
    }

    pub fn set_expanded(&mut self, group: Group, expanded: bool) {
        match group {
            Group::Changes => self.expanded_changes = expanded,
            Group::Unversioned => self.expanded_unversioned = expanded,
        }
        if !expanded {
            // A selected file of a collapsed group gives the selection to
            // the group.
            let files: HashSet<PathBuf> = self.files(group).into_iter().collect();
            let had = self
                .selected
                .iter()
                .any(|r| matches!(r, Row::File(p) if files.contains(p)));
            self.selected
                .retain(|r| !matches!(r, Row::File(p) if files.contains(p)));
            if had {
                self.selected.insert(Row::Group(group));
                self.lead = Some(Row::Group(group));
            }
        }
    }

    pub fn expand_all(&mut self) {
        self.expanded_changes = true;
        self.expanded_unversioned = true;
    }

    pub fn collapse_all(&mut self) {
        self.set_expanded(Group::Changes, false);
        self.set_expanded(Group::Unversioned, false);
    }

    /// The groups that have files, in order.
    pub fn groups(&self) -> Vec<Group> {
        let mut out = Vec::new();
        if !self.changes.is_empty() {
            out.push(Group::Changes);
        }
        if !self.unversioned.is_empty() {
            out.push(Group::Unversioned);
        }
        out
    }

    /// The rows shown, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for group in self.groups() {
            out.push(Row::Group(group));
            if self.is_expanded(group) {
                out.extend(self.files(group).into_iter().map(Row::File));
            }
        }
        out
    }

    // ---- inclusion --------------------------------------------------------

    pub fn is_included(&self, path: &Path) -> bool {
        self.included.contains(path)
    }

    /// The check box of a group.
    pub fn inclusion(&self, group: Group) -> Inclusion {
        let files = self.files(group);
        let n = files.iter().filter(|p| self.included.contains(*p)).count();
        match n {
            0 => Inclusion::None,
            n if n == files.len() => Inclusion::All,
            _ => Inclusion::Some,
        }
    }

    /// The check box of a row.
    pub fn row_inclusion(&self, row: &Row) -> Inclusion {
        match row {
            Row::Group(g) => self.inclusion(*g),
            Row::File(p) if self.included.contains(p) => Inclusion::All,
            Row::File(_) => Inclusion::None,
        }
    }

    /// The files of rows: a group stands for its files.
    pub fn files_of(&self, rows: &[Row]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for row in rows {
            match row {
                Row::Group(g) => out.extend(self.files(*g)),
                Row::File(p) => out.push(p.clone()),
            }
        }
        let mut seen = HashSet::new();
        out.retain(|p| seen.insert(p.clone()));
        out
    }

    /// Includes or excludes files.
    pub fn set_included(&mut self, paths: &[PathBuf], included: bool) {
        for p in paths {
            if self.group_of(p).is_none() {
                continue;
            }
            if included {
                self.included.insert(p.clone());
            } else {
                self.included.remove(p);
            }
        }
    }

    /// The check boxes of rows clicked or toggled with Space
    /// (`ChangesTree.toggleChanges`): all included when any was not, else
    /// all excluded.
    pub fn toggle(&mut self, rows: &[Row]) {
        let files = self.files_of(rows);
        let any_excluded = files.iter().any(|p| !self.included.contains(p));
        self.set_included(&files, any_excluded);
    }

    /// Includes every change (Commit…, ⌘K).
    pub fn include_all_changes(&mut self) {
        let paths = self.files(Group::Changes);
        self.set_included(&paths, true);
    }

    pub fn included_changes(&self) -> Vec<Change> {
        self.changes
            .iter()
            .filter(|c| self.included.contains(&c.path))
            .cloned()
            .collect()
    }

    pub fn included_unversioned(&self) -> Vec<PathBuf> {
        self.unversioned
            .iter()
            .filter(|p| self.included.contains(*p))
            .cloned()
            .collect()
    }

    // ---- selection --------------------------------------------------------

    pub fn is_selected(&self, row: &Row) -> bool {
        self.selected.contains(row)
    }

    pub fn lead(&self) -> Option<&Row> {
        self.lead.as_ref()
    }

    /// The selected rows, top to bottom.
    pub fn selected_rows(&self) -> Vec<Row> {
        self.rows()
            .into_iter()
            .filter(|r| self.selected.contains(r))
            .collect()
    }

    /// The files of the selection.
    pub fn selected_files(&self) -> Vec<PathBuf> {
        self.files_of(&self.selected_rows())
    }

    /// Selects one row.
    pub fn select(&mut self, row: &Row) {
        self.selected.clear();
        self.selected.insert(row.clone());
        self.lead = Some(row.clone());
        self.anchor = Some(row.clone());
    }

    /// Adds a row to the selection or takes it out (⌘-click).
    pub fn toggle_selected(&mut self, row: &Row) {
        if !self.selected.remove(row) {
            self.selected.insert(row.clone());
        }
        self.lead = Some(row.clone());
        self.anchor = Some(row.clone());
    }

    /// Selects the rows from the anchor to `row` (Shift-click).
    pub fn select_range(&mut self, row: &Row) {
        let rows = self.rows();
        let Some(to) = rows.iter().position(|r| r == row) else {
            return;
        };
        let from = self
            .anchor
            .as_ref()
            .and_then(|a| rows.iter().position(|r| r == a))
            .unwrap_or(to);
        let (a, b) = (from.min(to), from.max(to));
        self.selected = rows[a..=b].iter().cloned().collect();
        self.lead = Some(row.clone());
    }

    /// Selects the file at `path` (Select Opened File), expanding its group;
    /// `false` when the tree does not list it.
    pub fn select_path(&mut self, path: &Path) -> bool {
        let Some(group) = self.group_of(path) else {
            return false;
        };
        self.set_expanded(group, true);
        self.select(&Row::File(path.to_owned()));
        true
    }

    fn lead_index(&self, rows: &[Row]) -> Option<usize> {
        self.lead
            .as_ref()
            .and_then(|l| rows.iter().position(|r| r == l))
    }

    /// Up or Down: the row above or below the lead (the first row when
    /// nothing is selected).
    pub fn move_selection(&mut self, delta: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let at = match self.lead_index(&rows) {
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(&rows[at]);
    }

    /// Right: expands a collapsed group, else goes to the next row.
    pub fn right(&mut self) {
        match self.lead.clone() {
            Some(Row::Group(g)) if !self.is_expanded(g) => self.set_expanded(g, true),
            _ => self.move_selection(1),
        }
    }

    /// Left: collapses an expanded group, else goes to a file's group.
    pub fn left(&mut self) {
        match self.lead.clone() {
            Some(Row::Group(g)) if self.is_expanded(g) => self.set_expanded(g, false),
            Some(Row::File(p)) => {
                if let Some(g) = self.group_of(&p) {
                    self.select(&Row::Group(g));
                }
            }
            _ => {}
        }
    }
}

/// The flat order of files: by name with the case ignored, then by path.
fn by_name(a: &Path, b: &Path) -> std::cmp::Ordering {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    name(a).cmp(&name(b)).then_with(|| a.cmp(b))
}

/// What a file row shows after its name (`ChangesBrowserChangeNode`):
/// "- renamed from a.txt" for a rename in its folder, "- moved from
/// old/dir" for a move.
pub fn origin_text(change: &Change, root: &Path) -> Option<String> {
    if change.status != FileStatus::Renamed {
        return None;
    }
    let old = change.old_path.as_deref()?;
    if old.parent() == change.path.parent() {
        let name = old.file_name()?.to_string_lossy();
        Some(format!("- renamed from {name}"))
    } else {
        Some(format!("- moved from {}", parent_text(old, root)))
    }
}

/// A file's folder as the tree shows it (`VcsPresentablePath`): relative to
/// the work tree's root with the system's separators, the root as its
/// name.
pub fn parent_text(path: &Path, root: &Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    match parent.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR),
        Err(_) => parent.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/w/demo")
    }

    fn change(rel: &str, status: FileStatus) -> Change {
        Change {
            path: root().join(rel),
            status,
            old_path: None,
        }
    }

    fn tree() -> ChangesTree {
        let mut t = ChangesTree::default();
        t.set_changes(
            &root(),
            vec![
                change("demo_pkg/added.py", FileStatus::Added),
                change("main.py", FileStatus::Modified),
            ],
            vec![root().join("untracked.py"), root().join(".idea/vcs.xml")],
        );
        t
    }

    #[test]
    fn changes_are_included_and_unversioned_files_are_not() {
        let t = tree();
        assert_eq!(t.inclusion(Group::Changes), Inclusion::All);
        assert_eq!(t.inclusion(Group::Unversioned), Inclusion::None);
        assert_eq!(
            t.rows(),
            vec![
                Row::Group(Group::Changes),
                Row::File(root().join("demo_pkg/added.py")),
                Row::File(root().join("main.py")),
                Row::Group(Group::Unversioned),
            ]
        );
        assert!(t.is_selected(&Row::Group(Group::Changes)));
    }

    #[test]
    fn toggling_includes_any_excluded_else_excludes() {
        let mut t = tree();
        let main = Row::File(root().join("main.py"));
        t.toggle(std::slice::from_ref(&main));
        assert_eq!(t.inclusion(Group::Changes), Inclusion::Some);
        t.toggle(&[Row::Group(Group::Changes)]);
        assert_eq!(t.inclusion(Group::Changes), Inclusion::All);
        t.toggle(&[Row::Group(Group::Changes)]);
        assert_eq!(t.inclusion(Group::Changes), Inclusion::None);
        t.toggle(&[Row::Group(Group::Unversioned)]);
        assert_eq!(t.included_unversioned().len(), 2);
    }

    #[test]
    fn a_new_change_is_included_while_all_others_are() {
        let mut t = tree();
        let mut changes = t.changes().to_vec();
        changes.push(change("z.py", FileStatus::Modified));
        t.set_changes(&root(), changes.clone(), Vec::new());
        assert!(t.is_included(&root().join("z.py")));
        t.set_included(&[root().join("main.py")], false);
        changes.push(change("zz.py", FileStatus::Modified));
        t.set_changes(&root(), changes, Vec::new());
        assert!(!t.is_included(&root().join("zz.py")));
        assert!(!t.is_included(&root().join("main.py")));
    }

    #[test]
    fn keys_move_expand_and_collapse() {
        let mut t = tree();
        t.move_selection(1);
        assert_eq!(t.lead(), Some(&Row::File(root().join("demo_pkg/added.py"))));
        t.left();
        assert_eq!(t.lead(), Some(&Row::Group(Group::Changes)));
        t.left();
        assert!(!t.is_expanded(Group::Changes));
        assert_eq!(t.rows().len(), 2);
        t.right();
        assert!(t.is_expanded(Group::Changes));
        t.move_selection(10);
        assert_eq!(t.lead(), Some(&Row::Group(Group::Unversioned)));
        t.right();
        assert_eq!(t.rows().len(), 6);
        t.select_range(&Row::File(root().join("main.py")));
        assert_eq!(t.selected_rows().len(), 2);
        assert!(t.select_path(&root().join("untracked.py")));
        assert_eq!(t.selected_files(), vec![root().join("untracked.py")]);
    }

    #[test]
    fn files_go_by_name_with_the_case_ignored() {
        let mut t = ChangesTree::default();
        t.set_changes(
            &root(),
            Vec::new(),
            [
                "untracked.py",
                ".idea/vcs.xml",
                ".idea/b/Project_Default.xml",
                ".idea/a/profiles.xml",
            ]
            .iter()
            .map(|p| root().join(p))
            .collect(),
        );
        let names: Vec<String> = t
            .unversioned()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "profiles.xml",
                "Project_Default.xml",
                "untracked.py",
                "vcs.xml"
            ]
        );
    }

    #[test]
    fn rows_say_where_files_are_and_where_they_came_from() {
        let renamed = Change {
            path: root().join("notes.md"),
            status: FileStatus::Renamed,
            old_path: Some(root().join("notes.txt")),
        };
        assert_eq!(
            origin_text(&renamed, &root()).as_deref(),
            Some("- renamed from notes.txt")
        );
        let moved = Change {
            path: root().join("pkg/notes.md"),
            status: FileStatus::Renamed,
            old_path: Some(root().join("docs/old/notes.md")),
        };
        assert_eq!(
            origin_text(&moved, &root()).as_deref(),
            Some(format!("- moved from docs{}old", std::path::MAIN_SEPARATOR).as_str())
        );
        assert_eq!(parent_text(&root().join("main.py"), &root()), "demo");
        assert_eq!(
            parent_text(&root().join(".idea/inspectionProfiles/a.xml"), &root()),
            format!(".idea{}inspectionProfiles", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(file_count(1), "1 file");
        assert_eq!(file_count(7), "7 files");
    }
}
