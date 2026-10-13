//! What each kind of result finds: recently opened files, files and
//! folders (the index), actions (the commands) and text (`project-search`).
//! Each returns its rows in its own order with their weights, and whether
//! it has more than it was asked for.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use std::path::Path;

use file_index::{EXACT_MATCH_DEGREE, FileHit, FileIndex, GotoQuery, absolute_path, rank};
use name_match::{Matcher, MatchingMode};
use project_search::{FileSearch, Results};
use search::CancelToken;

use super::actions::{self, ActionItem};
use crate::app::NavigationHooks;
use crate::file_types::FileTypes;

/// A kind of result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Recent,
    Files,
    Actions,
    Text,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Recent,
        Provider::Files,
        Provider::Actions,
        Provider::Text,
    ];

    /// The name the All tab's filter lists.
    pub fn title(self) -> &'static str {
        match self {
            Provider::Recent => "Recent Files",
            Provider::Files => "Files",
            Provider::Actions => "Actions",
            Provider::Text => "Text",
        }
    }

    /// Kept on top of the All tab, whatever their weight.
    pub(super) fn pinned(self) -> bool {
        self == Provider::Recent
    }
}

/// A row of the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    File(FileRow),
    Action(usize),
    Text(TextRow),
    /// "… more": Enter or a click shows the next page.
    More,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub path: PathBuf,
    pub is_dir: bool,
    /// Not project content.
    pub non_project: bool,
    /// From the recent files.
    pub recent: bool,
    pub weight: i32,
    /// Shown after the name: the folder, or what the product's language
    /// makes of it (a Python package path).
    pub location: String,
    /// A folder that is a package of the product's language.
    pub package: bool,
}

/// Where rows are shown from: the project folder, its source roots and the
/// product's hooks.
#[derive(Clone, Default)]
pub(super) struct Places {
    pub root: Option<PathBuf>,
    pub source_roots: Arc<Vec<PathBuf>>,
    pub hooks: NavigationHooks,
    pub home: Option<PathBuf>,
}

impl Places {
    /// The location shown after a row's name: a folder's own path, a
    /// file's folder (or what the product's language makes of it), relative
    /// to the project folder; outside the project from the home folder.
    pub fn location(&self, path: &Path, is_dir: bool) -> String {
        let folder = if is_dir { Some(path) } else { path.parent() };
        let Some(folder) = folder else {
            return String::new();
        };
        let in_project = self
            .root
            .as_deref()
            .and_then(|root| folder.strip_prefix(root).ok());
        let Some(relative) = in_project else {
            return self.home_relative(folder);
        };
        if !is_dir && let Some(location) = self.hooks.file_location {
            let base = self
                .source_roots
                .iter()
                .filter(|r| folder.starts_with(r))
                .max_by_key(|r| r.components().count())
                .map(PathBuf::as_path)
                .or(self.root.as_deref());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if let Some(base) = base
                && let Ok(inner) = folder.strip_prefix(base)
                && let Some(text) = location(&name, &slashed(inner))
            {
                return text;
            }
        }
        slashed(relative)
    }

    /// A path for the footer: from its source root, else from the home
    /// folder.
    pub fn footer_path(&self, path: &Path) -> String {
        let root = self
            .source_roots
            .iter()
            .filter(|r| path.starts_with(r) && path != r.as_path())
            .max_by_key(|r| r.components().count());
        match root.and_then(|r| path.strip_prefix(r).ok()) {
            Some(inner) => slashed(inner),
            None => self.home_relative(path),
        }
    }

    fn home_relative(&self, path: &Path) -> String {
        match self.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
            Some(inner) => format!("~/{}", slashed(inner)),
            None => path.display().to_string(),
        }
    }
}

/// A relative path with `/` separators.
fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A line with matches.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRow {
    pub path: PathBuf,
    /// Zero-based line and the column of its first match.
    pub line: usize,
    pub column: usize,
    /// The line without its indentation, and the matches in it (char
    /// ranges).
    pub text: String,
    pub ranges: Vec<Range<usize>>,
}

/// What one provider found.
pub(super) struct Found {
    pub provider: Provider,
    pub rows: Vec<(Row, i32)>,
    pub more: bool,
}

/// Recently opened files whose names match (all of them for an empty
/// text), best match first, the more recent first between equal matches;
/// the file of the active editor is left out (deleted files are left out
/// of `files` beforehand, off the UI thread).
pub(super) fn recent(
    files: &[PathBuf],
    current: Option<&PathBuf>,
    text: &str,
    limit: usize,
    places: &Places,
) -> Found {
    let pattern = file_index::parse_position(text).pattern;
    let pattern = pattern.trim();
    let mut matcher = Matcher::new(&format!("*{pattern}"), MatchingMode::IgnoreCase);
    if !pattern.starts_with('*') {
        matcher = matcher.prefer_start();
    }
    let mut rows: Vec<(Row, i32)> = Vec::new();
    let mut seen = HashSet::new();
    for path in files {
        if Some(path) == current || !seen.insert(path) {
            continue;
        }
        let name: Vec<char> = path
            .file_name()
            .map(|n| n.to_string_lossy().chars().collect())
            .unwrap_or_default();
        let Some(m) = matcher.match_chars(&name) else {
            continue;
        };
        let degree = if pattern.is_empty() { 0 } else { m.degree };
        rows.push((
            Row::File(FileRow {
                path: path.clone(),
                is_dir: false,
                non_project: false,
                recent: true,
                weight: degree,
                location: places.location(path, false),
                package: false,
            }),
            degree,
        ));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    let more = rows.len() > limit;
    rows.truncate(limit);
    Found {
        provider: Provider::Recent,
        rows,
        more,
    }
}

/// The commands that match.
pub(super) fn actions(
    items: &[ActionItem],
    text: &str,
    include_disabled: bool,
    limit: usize,
) -> Found {
    let hits = actions::search(items, text, include_disabled);
    let more = hits.len() > limit;
    let rows = hits
        .into_iter()
        .take(limit)
        .map(|h| (Row::Action(h.index), h.weight))
        .collect();
    Found {
        provider: Provider::Actions,
        rows,
        more,
    }
}

/// What Go to File looks for, sent to a background thread.
pub(super) struct FilesQuery {
    pub index: Arc<FileIndex>,
    pub text: String,
    pub non_project: bool,
    pub open_files: Vec<PathBuf>,
    pub current: Option<PathBuf>,
    /// The home folder, for paths typed as `~/...`.
    pub home: Option<PathBuf>,
    /// File types left out by the filter ("Directory" for folders).
    pub hidden_types: HashSet<String>,
    pub types: FileTypes,
    pub limit: usize,
    pub places: Places,
}

/// The name a file's type has in the filter.
pub(super) const DIRECTORY_TYPE: &str = "Directory";

/// Files and folders, the first `limit` in the popup's order
/// ([`file_index::rank`]): a path typed in full or a file named as typed
/// first, then by where and how well the name matches.
pub(super) fn files(q: FilesQuery, cancel: &CancelToken) -> Found {
    let query = GotoQuery {
        text: &q.text,
        non_project: q.non_project,
        open_files: &q.open_files,
        context: q.current.as_deref(),
        home: q.home.as_deref(),
    };
    let hidden_path = |path: &Path, is_dir: bool| {
        if q.hidden_types.is_empty() {
            return false;
        }
        let name = if is_dir {
            DIRECTORY_TYPE
        } else {
            match q.types.for_path(path) {
                Some(t) => t.name,
                None => return false,
            }
        };
        q.hidden_types.contains(name)
    };
    let hidden = |h: &FileHit| hidden_path(&q.index.path(h.id), h.is_dir);
    // An absolute path outside the index, typed in full, goes first.
    let outside = absolute_path(&q.text, q.home.as_deref())
        .filter(|path| q.index.find(path).is_none())
        .map(|path| {
            let is_dir = path.is_dir();
            FileRow {
                location: q.places.location(&path, is_dir),
                path,
                is_dir,
                non_project: true,
                recent: false,
                weight: EXACT_MATCH_DEGREE,
                package: false,
            }
        })
        .filter(|row| !hidden_path(&row.path, row.is_dir));
    let mut hits: Vec<FileHit> = q
        .index
        .goto_file(&query, &|| cancel.is_cancelled())
        .into_iter()
        .filter(|h| !hidden(h))
        .collect();
    rank(&q.index, &query, &mut hits);
    let more = hits.len() > q.limit;
    hits.truncate(q.limit);
    let markers = q.places.hooks.package_markers;
    let mut rows: Vec<(Row, i32)> = Vec::with_capacity(hits.len() + 1);
    if let Some(row) = outside {
        rows.push((Row::File(row), EXACT_MATCH_DEGREE));
    }
    let typed = file_index::sanitized_pattern(&q.text);
    let typed = typed.rsplit('/').next().unwrap_or_default().trim();
    let named_as_typed = |name: &str| {
        !typed.is_empty()
            && (name == typed
                || !typed.contains('.')
                    && name.rsplit_once('.').is_some_and(|(stem, _)| stem == typed))
    };
    // Weights that never grow down the list, so that mixing the kinds on
    // All keeps this order.
    let mut ceiling = i32::MAX;
    for h in hits {
        let exact = h.exact || !h.is_dir && named_as_typed(q.index.name(h.id));
        let weight = if exact {
            h.degree.max(EXACT_MATCH_DEGREE)
        } else {
            h.degree
        };
        let weight = weight.min(ceiling);
        ceiling = weight;
        let path = q.index.path(h.id);
        let package = h.is_dir
            && markers
                .iter()
                .any(|m| q.index.find(&path.join(m)).is_some());
        rows.push((
            Row::File(FileRow {
                location: q.places.location(&path, h.is_dir),
                path,
                is_dir: h.is_dir,
                non_project: h.non_project,
                recent: false,
                weight,
                package,
            }),
            weight,
        ));
    }
    Found {
        provider: Provider::Files,
        rows,
        more,
    }
}

/// Lines with matches, up to `search.limit` matches, in the reference's
/// order of files; every one weighs 0 (it goes after the other kinds).
pub(super) fn text(search: FileSearch, cancel: &CancelToken) -> Found {
    let mut results = Results::new();
    let found = std::sync::Mutex::new(Vec::new());
    let summary = search.run(cancel, &|file| {
        if let Ok(mut f) = found.lock() {
            f.push(file);
        }
    });
    for file in found.into_inner().unwrap_or_default() {
        results.add(file);
    }
    let rows = results
        .rows()
        .iter()
        .map(|row| {
            let chars: Vec<char> = row.line.text.chars().collect();
            let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
            let text: String = chars[indent..]
                .iter()
                .map(|&c| if c == '\t' { ' ' } else { c })
                .collect();
            let shift = row.line.text_column + indent;
            let ranges = row
                .line
                .ranges
                .iter()
                .map(|r| r.start.saturating_sub(shift)..r.end.saturating_sub(shift))
                .filter(|r| r.end > r.start)
                .collect();
            (
                Row::Text(TextRow {
                    path: (*row.path).clone(),
                    line: row.line.line,
                    column: row.line.first.start.column,
                    text,
                    ranges,
                }),
                0,
            )
        })
        .collect();
    Found {
        provider: Provider::Text,
        rows,
        more: summary.truncated,
    }
}
