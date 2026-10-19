//! File operations of the project tool window (#37): new files and
//! folders, renaming and deleting, with the reference IDE's checks of new
//! names (`checkFileNameInput`, `CreateDirectoryOrPackageHandler`,
//! `RenameDialog`) and its messages (`IdeBundle`, `CoreBundle`,
//! `LangBundle`). Deleting moves to the system trash where the reference
//! does (`DeleteHandler`, `TrashBin`).
//!
//! Everything here is blocking: callers run it off the UI thread. A path
//! that would leave the project folder is refused; nothing outside it is
//! created, renamed or deleted.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::filter::Filter;

/// Why a name typed for a new file or folder, or a new name, cannot be
/// used. The messages are the reference IDE's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("A name must be specified")]
    Empty,
    #[error("'{0}' is an invalid file name")]
    InvalidFileName(String),
    #[error("'{0}' is an invalid directory name")]
    InvalidDirectoryName(String),
    #[error("A file with the name '{0}' already exists")]
    FileExists(String),
    #[error("A directory with the name '{0}' already exists")]
    DirectoryExists(String),
    /// The name is on the list of ignored files and folders: the result
    /// would not be shown.
    #[error(
        "Trying to create a directory with an ignored name ('{0}'); the result will not be visible"
    )]
    Ignored(String),
    /// The reference creates `../x` and `~/x` anywhere; Pyoxide never
    /// writes outside the project.
    #[error("'{0}' is outside the project")]
    OutsideProject(String),
    /// A new name that is no single file name (Rename).
    #[error("'{0}' is not a valid identifier")]
    NotValid(String),
    /// Rename onto an existing file or folder.
    #[error("File '{0}' already exists.")]
    FileAlreadyExists(String),
    #[error("Directory '{0}' already exists.")]
    DirectoryAlreadyExists(String),
}

/// A note about a new folder's name that does not stop its creation
/// (shown in the warning colour).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameWarning {
    #[error(
        "Trying to create a directory with an ignored name ('{0}'); the result will not be visible"
    )]
    Ignored(String),
    #[error(
        "Note: \".\" in the name is treated as a regular character. Use \"/\" instead if you mean to create nested directories"
    )]
    Dot,
    #[error(
        "Trying to create a package with an ignored name ('{0}'); the result will not be visible"
    )]
    IgnoredPackage(String),
}

/// What [`NewTarget`] creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    File,
    Directory,
}

/// Where a typed name leads: the folders to create on the way (outermost
/// first) and the new file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTarget {
    pub kind: NewKind,
    pub dirs: Vec<PathBuf>,
    pub path: PathBuf,
}

/// A change on disk made by an operation, for the tree, the file index,
/// the editor tabs and the language services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsChange {
    Created {
        path: PathBuf,
        is_dir: bool,
    },
    Removed {
        path: PathBuf,
        is_dir: bool,
    },
    /// Renamed or moved; a folder keeps its content.
    Renamed {
        from: PathBuf,
        to: PathBuf,
        is_dir: bool,
    },
}

/// The parts of a typed path: `/` separates folders (and `\` on Windows,
/// where the reference turns it into `/`); empty parts are skipped.
fn tokens(input: &str) -> Vec<&str> {
    input
        .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Checks a name typed in the New File or New Directory popup for the
/// folder `dir` and tells where it leads (the reference's
/// `checkFileNameInput` and `CreateDirectoryOrPackageHandler`): `a/b/c.py`
/// creates the folders `a` and `b` as needed, `..` goes up, `.` stays. An
/// existing file on the way, an existing file or folder at the end, an
/// ignored name for a file and a path that leaves `root` are errors.
pub fn check_new(
    dir: &Path,
    input: &str,
    kind: NewKind,
    root: &Path,
    filter: &Filter,
) -> Result<NewTarget, NameError> {
    let parts = tokens(input);
    let Some((&last, _)) = parts.split_last() else {
        return Err(NameError::Empty);
    };
    if last == "." || last == ".." {
        return Err(match kind {
            NewKind::File => NameError::InvalidFileName(last.to_owned()),
            NewKind::Directory => NameError::InvalidDirectoryName(last.to_owned()),
        });
    }
    if kind == NewKind::File && filter.is_hidden(last) {
        return Err(NameError::Ignored(last.to_owned()));
    }
    let mut current = dir.to_owned();
    // The folders on the way exist until one does not.
    let mut existing = true;
    let mut dirs = Vec::new();
    for (i, &part) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        match part {
            "." => continue,
            ".." => {
                if current == root || !current.starts_with(root) {
                    return Err(NameError::OutsideProject(input.to_owned()));
                }
                current.pop();
                existing = current.is_dir();
                continue;
            }
            "~" if i == 0 => return Err(NameError::OutsideProject(input.to_owned())),
            _ => {}
        }
        current.push(part);
        if existing {
            match std::fs::symlink_metadata(&current) {
                Ok(_) if !current.is_dir() => {
                    return Err(NameError::FileExists(part.to_owned()));
                }
                Ok(_) if is_last => return Err(NameError::DirectoryExists(part.to_owned())),
                Ok(_) => continue,
                Err(_) => existing = false,
            }
        }
        if !is_last {
            dirs.push(current.clone());
        }
    }
    if !current.starts_with(root) || current == root {
        return Err(NameError::OutsideProject(input.to_owned()));
    }
    Ok(NewTarget {
        kind,
        dirs,
        path: current,
    })
}

/// A warning about a valid new folder name: an ignored name on the way, or
/// a dot in a name without `/` (the reference's
/// `CreateDirectoryOrPackageHandler.checkForWarnings`).
pub fn new_directory_warning(input: &str, filter: &Filter) -> Option<NameWarning> {
    let parts = tokens(input);
    if let Some(ignored) = parts.iter().find(|t| filter.is_hidden(t)) {
        return Some(NameWarning::Ignored((*ignored).to_owned()));
    }
    let has_separator = input.contains('/') || (cfg!(windows) && input.contains('\\'));
    (input.contains('.') && !has_separator).then_some(NameWarning::Dot)
}

/// Creates what [`check_new`] found: the folders on the way, then the new
/// file (empty) or folder. A file or folder that appeared meanwhile is not
/// replaced. Returns the changes, outermost first.
pub fn create(target: &NewTarget) -> io::Result<Vec<FsChange>> {
    let mut changes = Vec::new();
    for dir in &target.dirs {
        match std::fs::create_dir(dir) {
            Ok(()) => changes.push(FsChange::Created {
                path: dir.clone(),
                is_dir: true,
            }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {}
            Err(e) => return Err(e),
        }
    }
    match target.kind {
        NewKind::File => {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target.path)?;
        }
        NewKind::Directory => std::fs::create_dir(&target.path)?,
    }
    changes.push(FsChange::Created {
        path: target.path.clone(),
        is_dir: target.kind == NewKind::Directory,
    });
    Ok(changes)
}

/// Writes `text` into a file that [`create`] made (a file from a template).
pub fn write_template(path: &Path, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// The name of a file from a template: `extension` is added to a name
/// without one (`CreateFileFromTemplateAction`); a name ending in a dot is
/// left alone.
pub fn with_extension(input: &str, extension: &str) -> String {
    let last = tokens(input).last().copied().unwrap_or_default();
    if extension.is_empty() || last.contains('.') {
        input.to_owned()
    } else {
        format!("{input}.{extension}")
    }
}

/// Checks a package path typed in the New Package popup for `dir`
/// (`CreateDirectoryOrPackageHandler` with `.` between the names): `a.b`
/// is the folder `b` in the folder `a`.
pub fn check_new_package(
    dir: &Path,
    input: &str,
    root: &Path,
    filter: &Filter,
) -> Result<NewTarget, NameError> {
    let parts: Vec<&str> = input.split('.').filter(|t| !t.is_empty()).collect();
    if let Some(bad) = parts
        .iter()
        .find(|t| t.contains('/') || (cfg!(windows) && t.contains('\\')))
    {
        return Err(NameError::InvalidDirectoryName((*bad).to_owned()));
    }
    check_new(dir, &parts.join("/"), NewKind::Directory, root, filter)
}

/// A warning about a valid package path: an ignored name on the way.
pub fn new_package_warning(input: &str, filter: &Filter) -> Option<NameWarning> {
    input
        .split('.')
        .find(|t| !t.is_empty() && filter.is_hidden(t))
        .map(|t| NameWarning::IgnoredPackage(t.to_owned()))
}

/// Creates a package that [`check_new_package`] found: the folders, then a
/// file `marker` with `text` in each folder from the new one up to `dir`
/// (not `dir` itself) that has none yet (`CreatePackageAction
/// .createInitPyInHierarchy`).
pub fn create_package(
    target: &NewTarget,
    dir: &Path,
    marker: &str,
    text: &str,
) -> io::Result<Vec<FsChange>> {
    let mut changes = create(target)?;
    let mut folder = target.path.clone();
    while folder != dir && folder.starts_with(dir) {
        let file = folder.join(marker);
        if std::fs::symlink_metadata(&file).is_err() {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?;
            write_template(&file, text)?;
            changes.push(FsChange::Created {
                path: file,
                is_dir: false,
            });
        }
        if !folder.pop() {
            break;
        }
    }
    Ok(changes)
}

/// The two names refer to the same file (a change of case only on a file
/// system that ignores case).
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// The two names refer to the same file (a change of case only on a file
/// system that ignores case): Windows resolves both to the name on disk.
#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    match (native_path::canonicalize(a), native_path::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Checks a new name for `path` (Rename): one file name, no folder
/// separators, and no other file or folder of that name next to it.
/// Returns the new path; the same name is no change (`Ok(path)`).
pub fn check_rename(path: &Path, new_name: &str, root: &Path) -> Result<PathBuf, NameError> {
    let name = new_name.trim();
    if !valid_file_name(name) {
        return Err(NameError::NotValid(name.to_owned()));
    }
    if path == root || !path.starts_with(root) {
        return Err(NameError::OutsideProject(path.display().to_string()));
    }
    let Some(parent) = path.parent() else {
        return Err(NameError::NotValid(name.to_owned()));
    };
    let to = parent.join(name);
    if to == path {
        return Ok(to);
    }
    if std::fs::symlink_metadata(&to).is_ok() && !same_file(path, &to) {
        return Err(if to.is_dir() {
            NameError::DirectoryAlreadyExists(name.to_owned())
        } else {
            NameError::FileAlreadyExists(name.to_owned())
        });
    }
    Ok(to)
}

/// One name that is neither empty, nor `.` or `..`, and has no folder
/// separator or NUL.
pub fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\0'])
        && !(cfg!(windows) && name.contains('\\'))
}

/// Renames `path` to `new_name` in the same folder after checking the name
/// again ([`check_rename`]). A change of case only renames on file systems
/// that ignore case.
pub fn rename(path: &Path, new_name: &str, root: &Path) -> io::Result<Option<FsChange>> {
    let to = check_rename(path, new_name, root).map_err(|e| io::Error::other(e.to_string()))?;
    if to == path {
        return Ok(None);
    }
    let is_dir = std::fs::symlink_metadata(path)?.is_dir();
    std::fs::rename(path, &to)?;
    Ok(Some(FsChange::Renamed {
        from: path.to_owned(),
        to,
        is_dir,
    }))
}

/// Copy or Move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move,
}

/// One file or folder to copy or move and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferItem {
    pub from: PathBuf,
    pub to: PathBuf,
    pub is_dir: bool,
    /// Another file of that name is there: the person chooses Overwrite or
    /// Skip (`SkipOverwriteChoice`).
    pub conflict: bool,
}

/// Why a copy or move cannot be done; the messages are the reference's
/// (`RefactoringBundle`, `MoveFilesOrDirectoriesUtil.checkIfMoveIntoSelf`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("No new name specified")]
    NoName,
    #[error("Name is not a valid file name")]
    InvalidName,
    #[error("No target directory specified")]
    NoTarget,
    #[error("Cannot create directory")]
    CannotCreateDirectory,
    #[error("Cannot place directory into itself.")]
    IntoItself,
    #[error("Cannot place directory into its subdirectory.")]
    IntoSubdirectory,
    #[error("Directory '{0}' already exists.")]
    DirectoryExists(String),
    #[error("File '{0}' already exists.")]
    FileExists(String),
    #[error("'{0}' is outside the project")]
    OutsideProject(String),
}

/// `path` with `.` and `..` resolved without looking at the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The folder typed in *To directory* (absolute, or relative to the
/// project). It must be in the project; missing folders are created when
/// the files go there (`DirectoryUtil.mkdirs`), but a file on the way is an
/// error.
pub fn target_dir(input: &str, root: &Path) -> Result<PathBuf, TransferError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(TransferError::NoTarget);
    }
    let path = Path::new(input);
    let path = normalize(&if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    });
    if !path.starts_with(root) {
        return Err(TransferError::OutsideProject(input.to_owned()));
    }
    let mut on_the_way = path.clone();
    loop {
        if let Ok(meta) = std::fs::metadata(&on_the_way) {
            if !meta.is_dir() {
                return Err(TransferError::CannotCreateDirectory);
            }
            break;
        }
        if !on_the_way.pop() {
            break;
        }
    }
    Ok(path)
}

/// Where `sources` go in `target` (with `new_name` for a single source),
/// checked as the reference checks a copy or move: a folder cannot go into
/// itself, a file of the same name is a conflict to ask about, a folder of
/// the same name is an error. A move to where the file is already is left
/// out.
pub fn plan_transfer(
    kind: TransferKind,
    sources: &[PathBuf],
    target: &Path,
    new_name: Option<&str>,
    root: &Path,
) -> Result<Vec<TransferItem>, TransferError> {
    if !target.starts_with(root) {
        return Err(TransferError::OutsideProject(target.display().to_string()));
    }
    let mut items = Vec::new();
    for from in outermost(sources) {
        if from == root || !from.starts_with(root) {
            return Err(TransferError::OutsideProject(from.display().to_string()));
        }
        let name = match new_name.filter(|_| sources.len() == 1) {
            Some(n) => n.trim().to_owned(),
            None => from
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        if name.is_empty() {
            return Err(TransferError::NoName);
        }
        if !valid_file_name(&name) {
            return Err(TransferError::InvalidName);
        }
        let is_dir = std::fs::symlink_metadata(&from).is_ok_and(|m| m.is_dir());
        if is_dir && target == from {
            return Err(TransferError::IntoItself);
        }
        if is_dir && target.starts_with(&from) {
            return Err(TransferError::IntoSubdirectory);
        }
        let to = target.join(&name);
        if to == from {
            match kind {
                TransferKind::Move => continue,
                TransferKind::Copy => return Err(TransferError::FileExists(name)),
            }
        }
        let mut conflict = false;
        if std::fs::symlink_metadata(&to).is_ok() && !same_file(&from, &to) {
            if is_dir || to.is_dir() {
                return Err(TransferError::DirectoryExists(name));
            }
            conflict = true;
        }
        items.push(TransferItem {
            from,
            to,
            is_dir,
            conflict,
        });
    }
    Ok(items)
}

/// Creates the folders up to `dir` that are missing.
fn make_dirs(dir: &Path, changes: &mut Vec<FsChange>) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut d = dir.to_owned();
    while std::fs::symlink_metadata(&d).is_err() {
        missing.push(d.clone());
        if !d.pop() {
            break;
        }
    }
    for d in missing.into_iter().rev() {
        std::fs::create_dir(&d)?;
        changes.push(FsChange::Created {
            path: d,
            is_dir: true,
        });
    }
    Ok(())
}

/// Copies a file or a folder with its content; a link is copied as a link.
fn copy_tree(from: &Path, to: &Path, changes: &mut Vec<FsChange>) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    #[cfg(unix)]
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
        changes.push(FsChange::Created {
            path: to.to_owned(),
            is_dir: from.is_dir(),
        });
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        changes.push(FsChange::Created {
            path: to.to_owned(),
            is_dir: true,
        });
        let mut children: Vec<_> = std::fs::read_dir(from)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            copy_tree(&child.path(), &to.join(child.file_name()), changes)?;
        }
    } else {
        std::fs::copy(from, to)?;
        changes.push(FsChange::Created {
            path: to.to_owned(),
            is_dir: false,
        });
    }
    Ok(())
}

/// Copies one planned item; `overwrite` replaces a conflicting file (else
/// the item is skipped). Folders on the way are created.
pub fn copy_item(item: &TransferItem, overwrite: bool) -> io::Result<Vec<FsChange>> {
    let mut changes = Vec::new();
    if item.conflict {
        if !overwrite {
            return Ok(changes);
        }
        std::fs::remove_file(&item.to)?;
        changes.push(FsChange::Removed {
            path: item.to.clone(),
            is_dir: false,
        });
    }
    if let Some(parent) = item.to.parent() {
        make_dirs(parent, &mut changes)?;
    }
    copy_tree(&item.from, &item.to, &mut changes)?;
    Ok(changes)
}

/// Moves one planned item; `overwrite` replaces a conflicting file (else
/// the item is skipped). A move to another volume copies and deletes.
pub fn move_item(item: &TransferItem, overwrite: bool) -> io::Result<Vec<FsChange>> {
    let mut changes = Vec::new();
    if item.conflict {
        if !overwrite {
            return Ok(changes);
        }
        std::fs::remove_file(&item.to)?;
        changes.push(FsChange::Removed {
            path: item.to.clone(),
            is_dir: false,
        });
    }
    if let Some(parent) = item.to.parent() {
        make_dirs(parent, &mut changes)?;
    }
    match std::fs::rename(&item.from, &item.to) {
        Ok(()) => {}
        Err(e) if crosses_devices(&e) => {
            let mut copied = Vec::new();
            copy_tree(&item.from, &item.to, &mut copied)?;
            if item.is_dir {
                std::fs::remove_dir_all(&item.from)?;
            } else {
                std::fs::remove_file(&item.from)?;
            }
        }
        Err(e) => return Err(e),
    }
    changes.push(FsChange::Renamed {
        from: item.from.clone(),
        to: item.to.clone(),
        is_dir: item.is_dir,
    });
    Ok(changes)
}

/// A rename failed because the target is on another volume (`EXDEV`,
/// `ERROR_NOT_SAME_DEVICE`).
fn crosses_devices(e: &io::Error) -> bool {
    let code = if cfg!(windows) { 17 } else { 18 };
    e.raw_os_error() == Some(code)
}

/// Paths without those inside another one of the list (the reference's
/// `PsiTreeUtil.filterAncestors`): deleting a folder deletes its content.
pub fn outermost(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if paths.iter().any(|q| q != p && p.starts_with(q)) || out.contains(p) {
            continue;
        }
        out.push(p.clone());
    }
    out
}

/// What a delete did with one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deleted {
    /// Moved to the system trash.
    Trashed,
    /// Removed for good (no trash for it, or not asked for).
    Removed,
}

/// Deletes one file or folder inside `root`: to the system trash when
/// `to_trash` and the trash takes it (the reference's
/// `TrashBin.canMoveToTrash`: the same volume as the home folder),
/// otherwise for good. A symbolic link is deleted, not its target.
pub fn delete(path: &Path, root: &Path, to_trash: bool) -> io::Result<Deleted> {
    if path == root || !path.starts_with(root) || has_parent_parts(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("'{}' is outside the project", path.display()),
        ));
    }
    let meta = std::fs::symlink_metadata(path)?;
    if to_trash && trash::can_move(path) {
        trash::move_to_trash(path)?;
        return Ok(Deleted::Trashed);
    }
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(Deleted::Removed)
}

fn has_parent_parts(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

/// The question of the Delete dialog for these entries (path, is a
/// folder), as the reference words it (`DeleteUtil.generateWarningMessage`
/// with `prompt.delete.elements`, and the warning about the content of
/// folders from `DeleteHandler`).
pub fn delete_prompt(items: &[(PathBuf, bool)]) -> String {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut text = match items {
        [(path, dir)] => {
            let kind = if *dir { "directory" } else { "file" };
            format!("Delete {kind} \"{}\"?", name(path))
        }
        _ => {
            let dirs = items.iter().filter(|(_, d)| *d).count();
            let files = items.len() - dirs;
            let mut parts = Vec::new();
            // The reference lists the kinds in the order of a hash map;
            // folders first reads best.
            if dirs > 0 {
                let word = if dirs == 1 {
                    "directory"
                } else {
                    "directories"
                };
                parts.push(format!("{dirs} {word}"));
            }
            if files > 0 {
                let word = if files == 1 { "file" } else { "files" };
                parts.push(format!("{files} {word}"));
            }
            format!("Delete {}?", parts.join(" and "))
        }
    };
    if let Some((path, _)) = items.iter().find(|(_, d)| *d) {
        if items.len() == 1 {
            text.push_str(&format!(
                "\nAll files and subdirectories in \"{}\" will be deleted.\nYou might not be able to fully undo this operation!",
                name(path)
            ));
        } else {
            text.push_str(
                "\nAll files and subdirectories in the selected directory(s) will be deleted.\nYou might not be able to fully undo this operation!",
            );
        }
    }
    text
}

/// The system trash (the reference's `TrashBin`, which uses the desktop's
/// trash and on other Unix systems the user's trash folder of the
/// freedesktop.org specification).
pub mod trash {
    use std::io;
    use std::path::Path;

    /// The trash takes `path`: it is on the same volume as the home folder
    /// (`TrashBin.canMoveToTrash`).
    pub fn can_move(path: &Path) -> bool {
        let Some(home) = home() else {
            return false;
        };
        same_volume(path, &home)
    }

    fn home() -> Option<std::path::PathBuf> {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        std::env::var_os(var).map(Into::into)
    }

    #[cfg(unix)]
    fn same_volume(a: &Path, b: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    }

    #[cfg(windows)]
    fn same_volume(a: &Path, b: &Path) -> bool {
        use std::path::Component;
        let prefix = |p: &Path| {
            native_path::canonicalize(p)
                .ok()
                .and_then(|p| match p.components().next() {
                    Some(Component::Prefix(prefix)) => {
                        Some(prefix.as_os_str().to_string_lossy().to_lowercase())
                    }
                    _ => None,
                })
        };
        matches!((prefix(a), prefix(b)), (Some(a), Some(b)) if a == b)
    }

    /// Moves a file or folder to the trash.
    pub fn move_to_trash(path: &Path) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        let context = {
            use ::trash::macos::{DeleteMethod, TrashContextExtMacos};
            let mut context = ::trash::TrashContext::default();
            // The file manager's own call; asking Finder would need the
            // permission to control it.
            context.set_delete_method(DeleteMethod::NsFileManager);
            context
        };
        #[cfg(not(target_os = "macos"))]
        let context = ::trash::TrashContext::default();
        context
            .delete(path)
            .map_err(|e| io::Error::other(format!("Cannot move to trash: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::AutoExclusions;
    use crate::store::Exclusions;

    fn filter(root: &Path) -> Filter {
        Filter::new(root, Exclusions::default(), AutoExclusions::default())
    }

    fn project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src/pkg")).unwrap();
        std::fs::write(root.join("src/app.py"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        (dir, root)
    }

    #[test]
    fn new_names_are_checked_as_the_reference_does() {
        let (_dir, root) = project();
        let f = filter(&root);
        let src = root.join("src");
        let check = |input: &str, kind| check_new(&src, input, kind, &root, &f);
        assert_eq!(check("", NewKind::File), Err(NameError::Empty));
        assert_eq!(check("//", NewKind::File), Err(NameError::Empty));
        assert_eq!(
            check("a/..", NewKind::File),
            Err(NameError::InvalidFileName("..".into()))
        );
        assert_eq!(
            check(".", NewKind::Directory),
            Err(NameError::InvalidDirectoryName(".".into()))
        );
        assert_eq!(
            check("app.py", NewKind::File),
            Err(NameError::FileExists("app.py".into()))
        );
        assert_eq!(
            check("app.py/x.py", NewKind::File),
            Err(NameError::FileExists("app.py".into()))
        );
        assert_eq!(
            check("pkg", NewKind::Directory),
            Err(NameError::DirectoryExists("pkg".into()))
        );
        assert_eq!(
            check("pkg", NewKind::File),
            Err(NameError::DirectoryExists("pkg".into()))
        );
        assert_eq!(
            check("x.pyc", NewKind::File),
            Err(NameError::Ignored("x.pyc".into()))
        );
        assert_eq!(
            check("../../x.py", NewKind::File),
            Err(NameError::OutsideProject("../../x.py".into()))
        );
        assert_eq!(
            check("~/x.py", NewKind::File),
            Err(NameError::OutsideProject("~/x.py".into()))
        );
        // A path through an existing folder and new ones.
        let t = check("pkg/a/b/c.py", NewKind::File).unwrap();
        assert_eq!(t.path, src.join("pkg/a/b/c.py"));
        assert_eq!(t.dirs, [src.join("pkg/a"), src.join("pkg/a/b")]);
        // `..` and `.` inside the project.
        let t = check("./../docs", NewKind::Directory).unwrap();
        assert_eq!(t.path, root.join("docs"));
        assert!(t.dirs.is_empty());
    }

    #[test]
    fn warnings_for_new_directories() {
        let (_dir, root) = project();
        let f = filter(&root);
        assert_eq!(new_directory_warning("a.b", &f), Some(NameWarning::Dot));
        assert_eq!(new_directory_warning("a/b.c", &f), None);
        assert_eq!(
            new_directory_warning("x/__pycache__", &f),
            Some(NameWarning::Ignored("__pycache__".into()))
        );
        assert_eq!(new_directory_warning("docs", &f), None);
    }

    #[test]
    fn creates_files_and_folders_without_replacing() {
        let (_dir, root) = project();
        let f = filter(&root);
        let t = check_new(&root, "a/b/c.py", NewKind::File, &root, &f).unwrap();
        let changes = create(&t).unwrap();
        assert_eq!(
            changes,
            [
                FsChange::Created {
                    path: root.join("a"),
                    is_dir: true
                },
                FsChange::Created {
                    path: root.join("a/b"),
                    is_dir: true
                },
                FsChange::Created {
                    path: root.join("a/b/c.py"),
                    is_dir: false
                },
            ]
        );
        assert!(root.join("a/b/c.py").is_file());
        // Created meanwhile: not replaced.
        std::fs::write(root.join("a/b/c.py"), "kept").unwrap();
        assert!(create(&t).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("a/b/c.py")).unwrap(),
            "kept"
        );
        let t = check_new(&root, "docs", NewKind::Directory, &root, &f).unwrap();
        create(&t).unwrap();
        assert!(root.join("docs").is_dir());
    }

    #[test]
    fn renames_with_the_reference_checks() {
        let (_dir, root) = project();
        let app = root.join("src/app.py");
        assert_eq!(
            check_rename(&app, "", &root),
            Err(NameError::NotValid(String::new()))
        );
        assert_eq!(
            check_rename(&app, "a/b.py", &root),
            Err(NameError::NotValid("a/b.py".into()))
        );
        assert_eq!(
            check_rename(&app, "pkg", &root),
            Err(NameError::DirectoryAlreadyExists("pkg".into()))
        );
        std::fs::write(root.join("src/b.py"), "").unwrap();
        assert_eq!(
            check_rename(&app, "b.py", &root),
            Err(NameError::FileAlreadyExists("b.py".into()))
        );
        assert_eq!(check_rename(&app, "app.py", &root), Ok(app.clone()));
        assert!(check_rename(&root, "x", &root).is_err());
        assert_eq!(rename(&app, " app.py ", &root).unwrap(), None);
        let change = rename(&app, "main.py", &root).unwrap();
        assert_eq!(
            change,
            Some(FsChange::Renamed {
                from: app.clone(),
                to: root.join("src/main.py"),
                is_dir: false
            })
        );
        assert!(!app.exists() && root.join("src/main.py").is_file());
        // A folder keeps its content.
        std::fs::write(root.join("src/pkg/m.py"), "m").unwrap();
        rename(&root.join("src/pkg"), "lib", &root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("src/lib/m.py")).unwrap(),
            "m"
        );
    }

    #[test]
    fn a_change_of_case_renames() {
        let (_dir, root) = project();
        let readme = root.join("README.md");
        let change = rename(&readme, "Readme.md", &root).unwrap();
        assert!(change.is_some());
        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"Readme.md".to_owned()), "{names:?}");
    }

    #[test]
    fn deletes_inside_the_project_only() {
        let (_dir, root) = project();
        assert!(delete(&root, &root, false).is_err());
        assert!(delete(&root.join("src/../../x"), &root, false).is_err());
        let outside = tempfile::tempdir().unwrap();
        assert!(delete(outside.path(), &root, false).is_err());
        assert!(outside.path().exists());
        assert_eq!(
            delete(&root.join("src"), &root, false).unwrap(),
            Deleted::Removed
        );
        assert!(!root.join("src").exists());
        assert_eq!(
            delete(&root.join("README.md"), &root, false).unwrap(),
            Deleted::Removed
        );
        assert!(delete(&root.join("README.md"), &root, false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_link_keeps_its_target() {
        let (_dir, root) = project();
        std::os::unix::fs::symlink(root.join("src"), root.join("link")).unwrap();
        delete(&root.join("link"), &root, false).unwrap();
        assert!(root.join("src/app.py").exists());
        assert!(std::fs::symlink_metadata(root.join("link")).is_err());
    }

    #[test]
    fn outermost_paths() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            outermost(&[p("/r/a/b"), p("/r/a"), p("/r/c"), p("/r/a")]),
            [p("/r/a"), p("/r/c")]
        );
    }

    #[test]
    fn delete_prompts() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            delete_prompt(&[(p("/r/app.py"), false)]),
            "Delete file \"app.py\"?"
        );
        assert_eq!(
            delete_prompt(&[(p("/r/src"), true)]),
            "Delete directory \"src\"?\nAll files and subdirectories in \"src\" will be deleted.\nYou might not be able to fully undo this operation!"
        );
        assert_eq!(
            delete_prompt(&[(p("/r/a.py"), false), (p("/r/b.py"), false)]),
            "Delete 2 files?"
        );
        assert_eq!(
            delete_prompt(&[(p("/r/a.py"), false), (p("/r/d"), true)]),
            "Delete 1 directory and 1 file?\nAll files and subdirectories in the selected directory(s) will be deleted.\nYou might not be able to fully undo this operation!"
        );
    }

    #[test]
    fn templates_and_packages() {
        let (_dir, root) = project();
        let f = filter(&root);
        assert_eq!(with_extension("a", "py"), "a.py");
        assert_eq!(with_extension("x/a.txt", "py"), "x/a.txt");
        assert_eq!(with_extension("a", ""), "a");
        let t = check_new(&root, &with_extension("t", "py"), NewKind::File, &root, &f).unwrap();
        create(&t).unwrap();
        write_template(&t.path, "import unittest\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("t.py")).unwrap(),
            "import unittest\n"
        );
        // `a.b` is a package in a package; each new folder gets a marker,
        // and so does an existing folder on the way, not the start.
        let src = root.join("src");
        assert_eq!(
            check_new_package(&src, "pkg", &root, &f),
            Err(NameError::DirectoryExists("pkg".into()))
        );
        assert_eq!(
            check_new_package(&src, "a/b", &root, &f),
            Err(NameError::InvalidDirectoryName("a/b".into()))
        );
        assert_eq!(
            new_package_warning("a.__pycache__", &f),
            Some(NameWarning::IgnoredPackage("__pycache__".into()))
        );
        let t = check_new_package(&src, "pkg.inner.deep", &root, &f).unwrap();
        let changes = create_package(&t, &src, "__init__.py", "").unwrap();
        for p in [
            "src/pkg/inner/deep/__init__.py",
            "src/pkg/inner/__init__.py",
            "src/pkg/__init__.py",
        ] {
            assert!(root.join(p).is_file(), "{p}");
            assert!(changes.contains(&FsChange::Created {
                path: root.join(p),
                is_dir: false
            }));
        }
        assert!(!root.join("src/__init__.py").exists());
    }

    #[test]
    fn target_folders_stay_in_the_project() {
        let (_dir, root) = project();
        assert_eq!(target_dir(" ", &root), Err(TransferError::NoTarget));
        assert_eq!(target_dir("src", &root), Ok(root.join("src")));
        assert_eq!(target_dir("src/new/../x", &root), Ok(root.join("src/x")));
        let abs = root.join("src/pkg").display().to_string();
        assert_eq!(target_dir(&abs, &root), Ok(root.join("src/pkg")));
        assert_eq!(
            target_dir("..", &root),
            Err(TransferError::OutsideProject("..".into()))
        );
        assert_eq!(
            target_dir("README.md/x", &root),
            Err(TransferError::CannotCreateDirectory)
        );
    }

    #[test]
    fn copies_and_moves_with_the_reference_checks() {
        let (_dir, root) = project();
        std::fs::write(root.join("src/pkg/n.py"), "n").unwrap();
        std::fs::write(root.join("src/app.py"), "app").unwrap();
        let src = root.join("src");
        let pkg = root.join("src/pkg");
        // A folder into itself or below it.
        assert_eq!(
            plan_transfer(
                TransferKind::Move,
                std::slice::from_ref(&src),
                &src,
                None,
                &root
            ),
            Err(TransferError::IntoItself)
        );
        assert_eq!(
            plan_transfer(
                TransferKind::Copy,
                std::slice::from_ref(&src),
                &pkg,
                None,
                &root
            ),
            Err(TransferError::IntoSubdirectory)
        );
        // Names.
        let app = src.join("app.py");
        assert_eq!(
            plan_transfer(
                TransferKind::Copy,
                std::slice::from_ref(&app),
                &root,
                Some(" "),
                &root
            ),
            Err(TransferError::NoName)
        );
        assert_eq!(
            plan_transfer(
                TransferKind::Copy,
                std::slice::from_ref(&app),
                &root,
                Some("a/b"),
                &root
            ),
            Err(TransferError::InvalidName)
        );
        assert_eq!(
            plan_transfer(
                TransferKind::Copy,
                std::slice::from_ref(&app),
                &src,
                None,
                &root
            ),
            Err(TransferError::FileExists("app.py".into()))
        );
        // A move where it is already does nothing.
        assert_eq!(
            plan_transfer(
                TransferKind::Move,
                std::slice::from_ref(&app),
                &src,
                None,
                &root
            ),
            Ok(Vec::new())
        );
        // Copy a folder with its content into a new folder.
        let items = plan_transfer(
            TransferKind::Copy,
            std::slice::from_ref(&pkg),
            &root.join("lib/new"),
            Some("copy"),
            &root,
        )
        .unwrap();
        let changes = copy_item(&items[0], false).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("lib/new/copy/n.py")).unwrap(),
            "n"
        );
        assert!(changes.contains(&FsChange::Created {
            path: root.join("lib"),
            is_dir: true
        }));
        assert!(changes.contains(&FsChange::Created {
            path: root.join("lib/new/copy/n.py"),
            is_dir: false
        }));
        // A file of the same name: a conflict, skipped or overwritten.
        std::fs::write(root.join("app.py"), "old").unwrap();
        let items = plan_transfer(
            TransferKind::Move,
            std::slice::from_ref(&app),
            &root,
            None,
            &root,
        )
        .unwrap();
        assert!(items[0].conflict);
        assert!(move_item(&items[0], false).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(root.join("app.py")).unwrap(), "old");
        let changes = move_item(&items[0], true).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("app.py")).unwrap(), "app");
        assert!(!app.exists());
        assert_eq!(
            changes,
            [
                FsChange::Removed {
                    path: root.join("app.py"),
                    is_dir: false
                },
                FsChange::Renamed {
                    from: app.clone(),
                    to: root.join("app.py"),
                    is_dir: false
                }
            ]
        );
        // A folder of the same name is an error.
        std::fs::create_dir(root.join("pkg")).unwrap();
        assert_eq!(
            plan_transfer(
                TransferKind::Move,
                std::slice::from_ref(&pkg),
                &root,
                None,
                &root
            ),
            Err(TransferError::DirectoryExists("pkg".into()))
        );
        // Nothing leaves the project.
        let outside = tempfile::tempdir().unwrap();
        assert!(matches!(
            plan_transfer(
                TransferKind::Copy,
                std::slice::from_ref(&pkg),
                outside.path(),
                None,
                &root
            ),
            Err(TransferError::OutsideProject(_))
        ));
    }

    /// Moves a file to the real trash of this machine; run by hand.
    #[test]
    #[ignore = "puts a file into the system trash"]
    fn moves_to_the_trash() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let file = root.join("pyoxide-trash-test.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(trash::can_move(&file));
        assert_eq!(delete(&file, &root, true).unwrap(), Deleted::Trashed);
        assert!(!file.exists());
    }
}
