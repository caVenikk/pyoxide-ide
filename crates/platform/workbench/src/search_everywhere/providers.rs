//! What each kind of result finds: recently opened files, files and
//! folders (the index), actions (the commands) and text (`project-search`).
//! Each returns its rows in its own order with their weights, and whether
//! it has more than it was asked for.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use file_index::{
    EXACT_MATCH_DEGREE, FileHit, FileIndex, GotoQuery, absolute_path, parse_position, rank,
};
use name_match::{Matcher, MatchingMode};
use project_search::{FileSearch, Results};
use search::CancelToken;

use super::actions::{self, ActionItem};
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
    /// More rows come as the list scrolls.
    Loading,
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
) -> Found {
    let pattern = parse_position(text).pattern;
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
}

/// The name a file's type has in the filter.
pub(super) const DIRECTORY_TYPE: &str = "Directory";

/// Files and folders, the first `limit` the reference finds, ordered by
/// weight; a file that matches the text exactly (its name, or its name
/// without the extension) comes before the other files.
pub(super) fn files(q: FilesQuery, cancel: &CancelToken) -> Found {
    let query = GotoQuery {
        text: &q.text,
        non_project: q.non_project,
        open_files: &q.open_files,
        context: q.current.as_deref(),
        home: q.home.as_deref(),
    };
    let hidden_path = |path: &std::path::Path, is_dir: bool| {
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
                path,
                is_dir,
                non_project: true,
                recent: false,
                weight: EXACT_MATCH_DEGREE,
            }
        })
        .filter(|row| !hidden_path(&row.path, row.is_dir));
    let hits: Vec<FileHit> = q
        .index
        .goto_file(&query, &|| cancel.is_cancelled())
        .into_iter()
        .filter(|h| !hidden(h))
        .collect();
    let more = hits.len() > q.limit;
    let mut page: Vec<FileHit> = hits.into_iter().take(q.limit).collect();
    rank(&mut page);
    // An exact match goes first among the files.
    let pattern = parse_position(&q.text).pattern.trim().to_owned();
    let top = page.first().map_or(0, |h| h.weight);
    let exact = |h: &FileHit| {
        if h.is_dir {
            return false;
        }
        if h.weight >= EXACT_MATCH_DEGREE {
            return true;
        }
        let name = q.index.name(h.id);
        name == pattern
            || !pattern.contains('.')
                && name
                    .rsplit_once('.')
                    .is_some_and(|(stem, _)| stem == pattern)
    };
    let rows = page
        .into_iter()
        .map(|h| {
            let weight = if exact(&h) {
                h.weight.max(top)
            } else {
                h.weight
            };
            (
                Row::File(FileRow {
                    path: q.index.path(h.id),
                    is_dir: h.is_dir,
                    non_project: h.non_project,
                    recent: false,
                    weight,
                }),
                weight,
            )
        })
        .collect::<Vec<_>>();
    let mut rows = rows;
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    if let Some(row) = outside {
        rows.insert(0, (Row::File(row), EXACT_MATCH_DEGREE));
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
