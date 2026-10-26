//! The index itself: every file and folder of the project by name, as a
//! tree of entries, built by one parallel walk of the project with the
//! project tree's rules ([`Filter`]).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ignore::{WalkBuilder, WalkState};
use project::Filter;

/// An entry of the index.
pub type EntryId = u32;

/// The project folder itself.
pub const ROOT: EntryId = 0;

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) name: u32,
    /// The folder it is in (the root's is the root).
    pub(crate) parent: EntryId,
    pub(crate) is_dir: bool,
    /// Excluded, or inside an excluded folder: not project content.
    pub(crate) excluded: bool,
    /// A settings file of the reference IDE (`.idea/`, `*.iml`): not project
    /// content either, but listed.
    pub(crate) config: bool,
    pub(crate) children: Vec<EntryId>,
    pub(crate) alive: bool,
    /// An excluded folder whose content is not listed.
    unlisted: bool,
}

/// A name and the entries that have it.
#[derive(Debug, Clone)]
pub(crate) struct Name {
    pub(crate) text: String,
    pub(crate) chars: Box<[char]>,
    pub(crate) entries: Vec<EntryId>,
}

/// File names of a project. Cheap to query from any thread; changes go to
/// a copy (see [`FileIndex::add`] and the others) or a new build.
#[derive(Debug, Clone)]
pub struct FileIndex {
    root: PathBuf,
    root_name: String,
    pub(crate) entries: Vec<Entry>,
    pub(crate) names: Vec<Name>,
    by_name: HashMap<String, u32>,
    /// The content of excluded folders is listed too.
    non_project: bool,
}

/// What a walk found: a path, a folder or not, excluded or not, a
/// settings file or not.
type Found = (PathBuf, bool, bool, bool);

impl FileIndex {
    /// An index with only the root folder.
    pub fn empty(root: &Path) -> Self {
        let root_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let mut index = Self {
            root: root.to_path_buf(),
            root_name: root_name.clone(),
            entries: Vec::new(),
            names: Vec::new(),
            by_name: HashMap::new(),
            non_project: false,
        };
        let name = index.intern(&root_name);
        index.entries.push(Entry {
            name,
            parent: ROOT,
            is_dir: true,
            excluded: false,
            config: false,
            children: Vec::new(),
            alive: true,
            unlisted: false,
        });
        index
    }

    /// Walks the project: hidden names (`.git`, `__pycache__`, ...) are left
    /// out, excluded folders are listed but their content only with
    /// `non_project`. `None` when cancelled.
    pub fn build(
        root: &Path,
        filter: &Filter,
        non_project: bool,
        cancel: &AtomicBool,
    ) -> Option<Self> {
        Self::build_pausable(root, filter, non_project, cancel, &|| {})
    }

    /// [`FileIndex::build`] that calls `pause` for every item it lists,
    /// from the walking threads: the work waits there while it is paused.
    pub fn build_pausable(
        root: &Path,
        filter: &Filter,
        non_project: bool,
        cancel: &AtomicBool,
        pause: &(dyn Fn() + Sync),
    ) -> Option<Self> {
        let mut index = Self::empty(root);
        index.non_project = non_project;
        let (found, temps) = walk(root, filter, non_project, cancel, pause)?;
        index.insert_found(found);
        remove_stale_save_temps(&temps, filter);
        Some(index)
    }

    /// Lists the content of the excluded folders too (non-project items).
    /// `None` when cancelled.
    pub fn with_non_project(&self, filter: &Filter, cancel: &AtomicBool) -> Option<Self> {
        let mut index = self.clone();
        if index.non_project {
            return Some(index);
        }
        let unlisted: Vec<EntryId> = (0..index.entries.len() as EntryId)
            .filter(|&id| {
                let e = &index.entries[id as usize];
                e.alive && e.unlisted
            })
            .collect();
        index.non_project = true;
        for id in unlisted {
            let dir = index.path(id);
            let (found, _) = walk(&dir, filter, true, cancel, &|| {})?;
            index.entries[id as usize].unlisted = false;
            index.insert_found(found);
        }
        Some(index)
    }

    /// Whether the content of excluded folders is listed.
    pub fn has_non_project(&self) -> bool {
        self.non_project
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The name of the project folder, the first part of
    /// [`Self::full_name`].
    pub fn root_name(&self) -> &str {
        &self.root_name
    }

    /// Files and folders in the index (project and non-project), without
    /// the root.
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.alive).count() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The files and folders that are project content.
    pub fn project_len(&self) -> usize {
        self.entries
            .iter()
            .skip(1)
            .filter(|e| e.alive && !e.excluded && !e.config)
            .count()
    }

    pub fn is_dir(&self, id: EntryId) -> bool {
        self.entries[id as usize].is_dir
    }

    /// Not project content: excluded, inside an excluded folder, or a
    /// settings file of the reference IDE.
    pub fn is_excluded(&self, id: EntryId) -> bool {
        let e = &self.entries[id as usize];
        e.excluded || e.config
    }

    pub fn name(&self, id: EntryId) -> &str {
        &self.names[self.entries[id as usize].name as usize].text
    }

    /// The path relative to the root, with `/` separators (empty for the
    /// root).
    pub fn relative(&self, id: EntryId) -> String {
        let mut parts = Vec::new();
        let mut id = id;
        while id != ROOT {
            parts.push(self.name(id));
            id = self.entries[id as usize].parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// The root folder's name and the relative path: `flask/src/app.py`.
    pub fn full_name(&self, id: EntryId) -> String {
        if id == ROOT {
            return self.root_name.clone();
        }
        format!("{}/{}", self.root_name, self.relative(id))
    }

    pub fn path(&self, id: EntryId) -> PathBuf {
        let mut path = self.root.clone();
        let rel = self.relative(id);
        if !rel.is_empty() {
            path.extend(rel.split('/'));
        }
        path
    }

    /// The entry of a path in the project, if it is in the index.
    pub fn find(&self, path: &Path) -> Option<EntryId> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut id = ROOT;
        for c in rel.components() {
            match c {
                Component::Normal(name) => {
                    let name = name.to_string_lossy();
                    id = self.child(id, &name)?;
                }
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(id)
    }

    fn child(&self, dir: EntryId, name: &str) -> Option<EntryId> {
        let entry = &self.entries[dir as usize];
        entry
            .children
            .iter()
            .copied()
            .find(|&c| self.entries[c as usize].alive && self.name(c) == name)
    }

    /// A file or folder was created (by the application, or seen on disk);
    /// its folders are added as needed. Hidden names are left out.
    pub fn add(&mut self, path: &Path, is_dir: bool, filter: &Filter) {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return;
        };
        let names: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let mut id = ROOT;
        let mut current = self.root.clone();
        for (i, name) in names.iter().enumerate() {
            if filter.is_hidden(name) {
                return;
            }
            current.push(name);
            let dir = i + 1 < names.len() || is_dir;
            id = match self.child(id, name) {
                Some(child) => child,
                None => {
                    let parent_excluded = self.entries[id as usize].excluded;
                    if parent_excluded && !self.non_project {
                        return;
                    }
                    let excluded = parent_excluded || filter.is_excluded(&current, dir);
                    let config = filter.is_project_config(&current);
                    self.push_entry(id, name, dir, excluded, config)
                }
            };
        }
    }

    /// A file or folder was deleted, with everything in it.
    pub fn remove(&mut self, path: &Path) {
        let Some(id) = self.find(path).filter(|&id| id != ROOT) else {
            return;
        };
        let parent = self.entries[id as usize].parent;
        self.entries[parent as usize].children.retain(|&c| c != id);
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let entry = &mut self.entries[id as usize];
            entry.alive = false;
            stack.append(&mut entry.children);
            let name = entry.name as usize;
            self.names[name].entries.retain(|&e| e != id);
        }
    }

    /// A file or folder was renamed or moved; a folder keeps its content.
    pub fn rename(&mut self, from: &Path, to: &Path, filter: &Filter) {
        let Some(id) = self.find(from).filter(|&id| id != ROOT) else {
            return;
        };
        let (Some(new_parent), Some(new_name)) = (to.parent(), to.file_name()) else {
            return;
        };
        let new_name = new_name.to_string_lossy().into_owned();
        if filter.is_hidden(&new_name) || !to.starts_with(&self.root) {
            self.remove(from);
            return;
        }
        if self.find(new_parent).is_none() {
            self.add(new_parent, true, filter);
        }
        let Some(parent) = self.find(new_parent) else {
            self.remove(from);
            return;
        };
        let old_parent = self.entries[id as usize].parent;
        self.entries[old_parent as usize]
            .children
            .retain(|&c| c != id);
        self.entries[parent as usize].children.push(id);
        let old_name = self.entries[id as usize].name as usize;
        self.names[old_name].entries.retain(|&e| e != id);
        let name = self.intern(&new_name);
        self.names[name as usize].entries.push(id);
        let entry = &mut self.entries[id as usize];
        entry.parent = parent;
        entry.name = name;
        // The exclusion follows the new place.
        let mut stack = vec![(id, to.to_path_buf())];
        while let Some((id, path)) = stack.pop() {
            let parent = self.entries[id as usize].parent;
            let is_dir = self.entries[id as usize].is_dir;
            let excluded = self.entries[parent as usize].excluded && parent != ROOT
                || filter.is_excluded(&path, is_dir);
            self.entries[id as usize].excluded = excluded;
            self.entries[id as usize].config = filter.is_project_config(&path);
            for &child in &self.entries[id as usize].children {
                stack.push((child, path.join(self.name(child))));
            }
        }
    }

    fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(Name {
            text: name.to_owned(),
            chars: name.chars().collect(),
            entries: Vec::new(),
        });
        self.by_name.insert(name.to_owned(), id);
        id
    }

    fn push_entry(
        &mut self,
        parent: EntryId,
        name: &str,
        is_dir: bool,
        excluded: bool,
        config: bool,
    ) -> EntryId {
        let name = self.intern(name);
        let id = self.entries.len() as EntryId;
        self.entries.push(Entry {
            name,
            parent,
            is_dir,
            excluded,
            config,
            children: Vec::new(),
            alive: true,
            unlisted: is_dir && excluded && !self.non_project,
        });
        self.entries[parent as usize].children.push(id);
        self.names[name as usize].entries.push(id);
        id
    }

    /// Adds what a walk found (in any order).
    fn insert_found(&mut self, mut found: Vec<Found>) {
        // Folders before their content.
        found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut ids: HashMap<PathBuf, EntryId> = HashMap::new();
        for (path, is_dir, excluded, config) in found {
            let parent = match path.parent() {
                Some(p) if p == self.root => ROOT,
                Some(p) => match ids.get(p).copied().or_else(|| self.find(p)) {
                    Some(id) => id,
                    None => continue,
                },
                None => continue,
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if self.child(parent, &name).is_some() {
                continue;
            }
            let id = self.push_entry(parent, &name, is_dir, excluded, config);
            if is_dir {
                ids.insert(path, id);
            }
        }
    }

    /// The live entries of a name.
    pub(crate) fn entries_of(&self, name: u32) -> &[EntryId] {
        &self.names[name as usize].entries
    }
}

/// Temporary files of an interrupted save the walk met (hidden like any
/// ignored name).
type SaveTemps = Vec<PathBuf>;

/// Removes the temporary files of saves that a session which is gone left
/// behind (a crash). Those in excluded folders (libraries, environments)
/// are not ours to touch; a file of a running process is kept. Blocking, but
/// the walk was just done in the background.
fn remove_stale_save_temps(temps: &[PathBuf], filter: &Filter) {
    for path in temps {
        if !filter.is_excluded(path, false) {
            project::remove_stale_save_temp(path);
        }
    }
}

/// Walks a folder in parallel: hidden names are left out; an excluded
/// folder is listed, its content only with `non_project`. The temporary
/// files of saves are left out too and returned.
fn walk(
    dir: &Path,
    filter: &Filter,
    non_project: bool,
    cancel: &AtomicBool,
    pause: &(dyn Fn() + Sync),
) -> Option<(Vec<Found>, SaveTemps)> {
    let found = Mutex::new(Vec::new());
    let temps = Arc::new(Mutex::new(Vec::new()));
    let mut builder = WalkBuilder::new(dir);
    builder.standard_filters(false).follow_links(false);
    let hidden = filter.clone();
    let temps_seen = Arc::clone(&temps);
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        if project::is_save_temp(&name) {
            if let Ok(mut temps) = temps_seen.lock() {
                temps.push(entry.path().to_owned());
            }
            return false;
        }
        !hidden.is_hidden(&name)
    });
    builder.build_parallel().run(|| {
        Box::new(|entry| {
            pause();
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if entry.depth() == 0 {
                return WalkState::Continue;
            }
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            let excluded = filter.is_excluded(entry.path(), is_dir);
            let config = filter.is_project_config(entry.path());
            let skip = is_dir && excluded && !non_project;
            if let Ok(mut found) = found.lock() {
                found.push((entry.into_path(), is_dir, excluded, config));
            }
            if skip {
                WalkState::Skip
            } else {
                WalkState::Continue
            }
        })
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let temps = std::mem::take(&mut *temps.lock().ok()?);
    Some((found.into_inner().ok()?, temps))
}
