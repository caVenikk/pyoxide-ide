//! Find in Files and Replace in Files: a movable, resizable popup over the
//! main window with a search field (Match Case, Words, Regex), a replace
//! field, a file mask, the scope (the project, a folder, or a named scope
//! such as the open files), the matching lines and an editor on the
//! selected one's file (the preview), as in the reference IDE
//! (tests/scenarios/editor/find-in-files.md).
//!
//! The search runs in the background (`project-search`): every change of
//! the query, the options, the mask or the scope waits [`DEBOUNCE`] for the
//! next one, then cancels the running search and starts a new one. Files
//! with matches stream in and are shown at once, ordered by [`Results`];
//! the rows of the previous search stay until the first new row arrives.
//! Like the reference, a search stops after [`PAGE_SIZE`] matches (the
//! counter then reads `100+ matches in 12+ files`) and scrolling to the end
//! of the list loads [`PAGE_SIZE`]` / 2` more.
//!
//! Enter opens the selected line in an editor at its match and closes the
//! popup unless it is pinned; in Replace in Files it replaces the selected
//! match. Up and Down move through the rows while the focus stays in the
//! field. The popup closes when the focus moves elsewhere in the window,
//! unless it is pinned. The file mask, its check box, the options, the
//! folders and masks used, the scope, the pin, the popup's place and size
//! and the split with the preview are remembered across sessions
//! (`find.toml` in the configuration folder), as the reference remembers
//! them in its application settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use editor::Editor;
use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, ScrollStrategy, Subscription, Task, UniformListScrollHandle, Window,
    prelude::*, px,
};
use project_search::{FileMask, FileSearch, Results, Scope};
use search::{CancelToken, Query, QueryError, SearchOptions};
use serde_json::json;
use text::{Buffer, Point, Rope};
use themes::Metric;
use ui::theme::ThemeExt;

use crate::file_types::FileTypes;
use crate::popup::PopupFrame;

mod dropdown;
mod preview;
mod replace;
mod settings;
mod view;

pub(crate) use settings::update_settings;
pub use settings::{FindInFilesSettings, init, load_settings, settings, settings_file};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the Find in Files popup.
    Open => "find-in-files.open", "Find in Files…",
        "Opens the Find in Files popup with the selected text, or the last search, selected in its field and searches the project.";
    /// Open the popup with the replace field.
    OpenReplace => "find-in-files.open-replace", "Replace in Files…",
        "Opens the popup in Replace in Files mode: a replace field, one row per match, Replace and Replace All.";
    /// Search for a text.
    Search {
        text: String,
        case_sensitive: Option<bool>,
        whole_words: Option<bool>,
        regex: Option<bool>,
        mask: Option<String>,
        replace: Option<String>,
        scope: Option<String>,
        directory: Option<String>,
        recursive: Option<bool>,
    } => "find-in-files.search", "Find in Files for Text",
        "Opens the popup and searches for a text; options that are given are switched on or off, a mask narrows the files (an empty mask switches it off), a replacement opens Replace in Files, a scope is `project`, `directory` (with `directory` and `recursive`) or the name of a scope such as `Open Files`.",
        r#"{"text": "def", "case_sensitive": false, "whole_words": false, "regex": false, "mask": "*.py", "scope": "directory", "directory": "src", "recursive": true}"#;
    /// Describe the popup.
    State => "find-in-files.state", "Find in Files State",
        "Returns the state of the Find in Files popup as JSON (query, options, mask, scope, status, rows, selected row, preview).";
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "FindInFiles";
    /// Close the popup.
    Close => "find-in-files.close", "Close", "Closes the Find in Files popup, also when it is pinned.";
    /// Select the next row.
    NextResult => "find-in-files.next-result", "Next Result",
        "Selects the next line of the results.";
    /// Select the previous row.
    PreviousResult => "find-in-files.previous-result", "Previous Result",
        "Selects the previous line of the results.";
    /// Down in the search field.
    Down => "find-in-files.down", "Next Result or Line Down",
        "Selects the next line of the results; while the search field holds several lines, moves the caret down instead.";
    /// Up in the search field.
    Up => "find-in-files.up", "Previous Result or Line Up",
        "Selects the previous line of the results; while the search field holds several lines, moves the caret up instead.";
    /// A page down in the results.
    PageDown => "find-in-files.page-down", "Results Page Down",
        "Selects the line of the results a page further down.";
    /// A page up in the results.
    PageUp => "find-in-files.page-up", "Results Page Up",
        "Selects the line of the results a page further up.";
    /// Open the selected row.
    OpenResult => "find-in-files.open-result", "Open Result",
        "Opens the file of the selected line at its match and closes the popup unless it is pinned.";
    /// Replace the selected match.
    ReplaceSelected => "find-in-files.replace-selected", "Replace",
        "Replaces the selected match in Replace in Files; the popup stays open.";
    /// Replace every match of the scope.
    ReplaceAll => "find-in-files.replace-all", "Replace All",
        "Replaces every match in the scope (after a confirmation when there are several files) and closes the popup.";
    /// File mask on or off.
    ToggleMask => "find-in-files.toggle-mask", "File Mask",
        "Switches the file mask on (and puts the focus in it) or off.";
    /// Keep the popup open.
    TogglePin => "find-in-files.toggle-pin", "Pin Window",
        "Keeps the popup open when the focus moves elsewhere or a result is opened.";
    /// Search the project.
    ScopeProject => "find-in-files.scope-project", "In Project",
        "Searches the whole project.";
    /// Search a module.
    ScopeModule => "find-in-files.scope-module", "Module",
        "Searches the module chosen in the module list (a project is one module).";
    /// Search a folder.
    ScopeDirectory => "find-in-files.scope-directory", "Directory",
        "Searches the folder in the directory field (and its subfolders when Recursively is on).";
    /// Search a named scope.
    ScopeNamed => "find-in-files.scope-named", "Scope",
        "Searches the scope chosen in the scope list (open files, recently viewed files, ...).";
    /// Subfolders of the directory on or off.
    ToggleRecursive => "find-in-files.toggle-recursive", "Search recursively in subdirectories",
        "Switches searching the subfolders of the Directory scope on or off.";
    /// Choose the folder.
    BrowseDirectory => "find-in-files.browse-directory", "Browse",
        "Shows a folder chooser for the Directory scope.";
    /// The next field.
    NextField => "find-in-files.next-field", "Next Field",
        "Moves the focus to the next field of the popup (search, replace, mask, folder).";
    /// The previous field.
    PreviousField => "find-in-files.previous-field", "Previous Field",
        "Moves the focus to the previous field of the popup.";
    /// Edit in the preview.
    FocusPreview => "find-in-files.focus-preview", "Go to Preview",
        "Moves the focus into the preview's editor (as a click into it does).";
    /// Open the file in an editor.
    EditSource => "find-in-files.edit-source", "Edit Source",
        "Opens the file in an editor: from the preview at its caret, else the selected line at its match; the popup closes unless it is pinned.";
}

/// Matches one search collects before it stops (the reference IDE's
/// `ide.usages.page.size`).
pub const PAGE_SIZE: usize = 100;
/// Wait after a change before searching again (the reference's delay).
pub const DEBOUNCE: Duration = Duration::from_millis(100);
/// Wait after the selection moved before the preview shows it.
pub const PREVIEW_DELAY: Duration = Duration::from_millis(50);
/// Most recent file masks and folders kept.
pub const RECENT_MASKS: usize = 300;
/// Files of the recently viewed and recently changed scopes.
pub const RECENT_FILES: usize = 50;
/// Distance of the popup from the top of the window, as in the reference.
const TOP: f32 = 115.0;
/// Default size of the popup.
const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 640.0;
/// Smallest size it can be resized to.
const MIN_WIDTH: f32 = 480.0;
const MIN_HEIGHT: f32 = 400.0;
const HEADER_HEIGHT: f32 = 40.0;
const SCOPE_HEIGHT: f32 = 36.0;
/// Height of a search field for one line.
const FIELD_HEIGHT: f32 = 36.0;
const PAD: f32 = 12.0;
/// Width of the file mask field.
const MASK_WIDTH: f32 = 140.0;
/// Rows a page up or down moves.
const PAGE_ROWS: usize = 10;
/// Share of the results list in the split with the preview (the
/// reference's default).
const SPLIT: f32 = 0.33;

/// What the popup tells the workbench.
#[derive(Debug, Clone)]
pub enum FindInFilesEvent {
    /// Open a file with the caret at a point; `document` is the file's
    /// text as edited in the preview, for the new tab to continue with.
    OpenAt {
        path: PathBuf,
        point: Point,
        document: Option<Buffer>,
    },
    /// The popup was closed with Escape: the focus goes back to where it
    /// was.
    Dismissed,
    /// A message for the status bar (replacements made, files that could
    /// not be changed).
    Message(String),
}

impl EventEmitter<FindInFilesEvent> for FindInFiles {}

/// What the popup is opened with.
#[derive(Clone, Default)]
pub struct OpenContext {
    pub root: Option<PathBuf>,
    /// The project's hide and exclude rules as the project tool window has
    /// them (`None`: the rules of the root).
    pub filter: Option<project::Filter>,
    /// Text selected in the active editor.
    pub selection: Option<String>,
    /// The search of the active editor's open find bar.
    pub find_bar_text: Option<String>,
    /// The editors of the open files, in tab order.
    pub editors: Vec<(PathBuf, Entity<Editor>)>,
    /// The active editor's file.
    pub current_file: Option<PathBuf>,
    /// Files shown in editors, most recent first.
    pub recently_viewed: Vec<PathBuf>,
    /// Files edited in this session, most recent first.
    pub recently_changed: Vec<PathBuf>,
    /// The folder selected in the project tool window when it had the
    /// focus (a selected file gives its folder).
    pub tree_folder: Option<PathBuf>,
}

/// Where a search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// The whole project.
    Project,
    /// A module: a Pyoxide project is one module, the project itself.
    Module,
    /// A folder, with or without its subfolders.
    Directory,
    /// A named scope.
    Named,
}

/// The scopes of the Scope tab, in the reference's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedScope {
    ProjectFiles,
    RecentlyViewed,
    RecentlyChanged,
    OpenFiles,
    CurrentFile,
}

impl NamedScope {
    pub const ALL: [NamedScope; 5] = [
        NamedScope::ProjectFiles,
        NamedScope::RecentlyViewed,
        NamedScope::RecentlyChanged,
        NamedScope::OpenFiles,
        NamedScope::CurrentFile,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NamedScope::ProjectFiles => "Project Files",
            NamedScope::RecentlyViewed => "Recently Viewed Files",
            NamedScope::RecentlyChanged => "Recently Changed Files",
            NamedScope::OpenFiles => "Open Files",
            NamedScope::CurrentFile => "Current File",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Where a search stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// Nothing to search for.
    Empty,
    /// Waiting for more typing, or searching.
    Searching,
    Done,
    /// The query, the mask or the folder cannot be used.
    Invalid(String),
    /// No project is open.
    NoProject,
}

/// Which field a message or a list belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Query,
    Replace,
    Mask,
    Directory,
}

/// The Find in Files popup.
pub struct FindInFiles {
    focus: FocusHandle,
    query: Entity<Editor>,
    replace: Entity<Editor>,
    mask: Entity<Editor>,
    directory: Entity<Editor>,
    open: bool,
    replace_mode: bool,
    options: SearchOptions,
    mask_on: bool,
    pinned: bool,
    scope: ScopeKind,
    named: NamedScope,
    recursive: bool,
    context: OpenContext,
    results: Results,
    /// The rows belong to the previous search; they go when the first row
    /// of the new one arrives.
    stale: bool,
    selected: Option<usize>,
    status: Status,
    /// The field whose text is at fault.
    invalid_field: Option<Field>,
    /// The search stopped at the limit.
    truncated: bool,
    /// Files skipped as too large by the last search.
    large_files: usize,
    /// Project configuration files whose matches the last search held back.
    held_back: usize,
    /// The last search started by a change, for loading more of it and for
    /// replacing.
    last_search: Option<FileSearch>,
    /// A pass loading more results runs.
    loading_more: bool,
    /// When the last search started, and how long it took to its first
    /// rows and to its end (measurements).
    started: Option<std::time::Instant>,
    first_rows: Option<Duration>,
    finished: Option<Duration>,
    generation: u64,
    cancel: Option<CancelToken>,
    search_task: Option<Task<()>>,
    debounce: Option<Task<()>>,
    list: UniformListScrollHandle,
    /// Place and size, moved by the header and resized by the edges.
    frame: PopupFrame,
    /// A list below a field (history, masks, folders, scopes).
    dropdown: Option<dropdown::Dropdown>,
    dropdown_focus: FocusHandle,
    preview: preview::Preview,
    /// Share of the list in the split with the preview, and the split's
    /// drag in progress.
    split: f32,
    split_drag: crate::popup::HeldDrag<(gpui::Pixels, f32)>,
    split_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    /// A replacement runs in the background.
    replace_task: Option<Task<()>>,
    /// A folder chooser is open.
    browsing: bool,
    /// The main window, for the editors of open files.
    host: Option<gpui::WeakEntity<crate::workbench::Workbench>>,
    /// Folder choosers asked for (the test platform has none to show).
    browse_requests: usize,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for FindInFiles {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FindInFiles {
    pub fn new(file_types: Rc<FileTypes>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut e = Editor::new(Buffer::new(""), window, cx);
                e.set_single_line(true);
                e.set_ui_font(true);
                e.set_transparent(true);
                // "Search" and "Replace" show while the field has no focus,
                // as in the reference (an empty field with the focus shows
                // the caret only).
                e.set_placeholder(Some(placeholder.into()));
                e.set_placeholder_unfocused(true);
                e.set_config(editor::EditorConfig {
                    brackets: Vec::new(),
                    ..editor::EditorConfig::default()
                });
                e
            })
        };
        let query = field("Search", window, cx);
        let replace = field("Replace", window, cx);
        let mask = field("", window, cx);
        let directory = field("", window, cx);
        let focus = cx.focus_handle();
        let dropdown_focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) {
                    this.schedule(cx);
                }
            }),
            cx.subscribe(&replace, |this, field, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) {
                    field.update(cx, |f, cx| f.set_error(false, cx));
                    let text = field.read(cx).text();
                    editor::find::set_last_replace(cx, &text);
                    this.preview_selected(cx);
                }
            }),
            cx.subscribe(&mask, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) && this.mask_on {
                    this.schedule(cx);
                }
            }),
            cx.subscribe(&directory, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_))
                    && this.scope == ScopeKind::Directory
                {
                    this.schedule(cx);
                }
            }),
            // Clicking elsewhere in the window closes the popup, unless it
            // is pinned; switching to another application does not.
            cx.on_focus_out(&focus, window, |this, _, window, cx| {
                if this.open && !this.pinned && !this.browsing && window.is_window_active() {
                    this.close(false, window, cx);
                }
            }),
            cx.on_blur(&dropdown_focus, window, |this, _, cx| {
                if this.dropdown.take().is_some() {
                    cx.notify();
                }
            }),
        ];
        let settings = settings(cx);
        let mut frame = PopupFrame::new(
            gpui::size(px(WIDTH), px(HEIGHT)),
            gpui::size(px(MIN_WIDTH), px(MIN_HEIGHT)),
            px(TOP),
        );
        if let Some(place) = settings.popup {
            frame.restore(place);
        }
        let shown_mask = settings.shown_mask();
        set_field_text(&mask, &shown_mask, cx);
        if let Some(dir) = settings.recent_directories.first() {
            set_field_text(&directory, dir, cx);
        }
        let named = settings
            .search_scope
            .as_deref()
            .and_then(NamedScope::from_name)
            .unwrap_or(NamedScope::ProjectFiles);
        let preview = preview::Preview::new(file_types, window, cx);
        Self {
            focus,
            query,
            replace,
            mask,
            directory,
            open: false,
            replace_mode: false,
            options: settings.options(),
            mask_on: settings.file_mask.is_some(),
            pinned: settings.pinned,
            scope: ScopeKind::Project,
            named,
            recursive: settings.recursive,
            context: OpenContext::default(),
            results: Results::new(),
            stale: false,
            selected: None,
            status: Status::Empty,
            invalid_field: None,
            truncated: false,
            large_files: 0,
            held_back: 0,
            last_search: None,
            loading_more: false,
            started: None,
            first_rows: None,
            finished: None,
            generation: 0,
            cancel: None,
            search_task: None,
            debounce: None,
            list: UniformListScrollHandle::new(),
            frame,
            dropdown: None,
            dropdown_focus,
            preview,
            split: settings.splitter.unwrap_or(SPLIT).clamp(0.1, 0.9),
            split_drag: Default::default(),
            split_bounds: None,
            replace_task: None,
            browsing: false,
            host: None,
            browse_requests: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The main window the popup belongs to (its tabs' editors share
    /// their documents with the preview).
    pub fn set_host(
        &mut self,
        host: gpui::WeakEntity<crate::workbench::Workbench>,
        cx: &mut Context<Self>,
    ) {
        self.preview.set_host(host.clone(), cx);
        self.host = Some(host);
    }

    // ---- state ------------------------------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_replace_mode(&self) -> bool {
        self.replace_mode
    }

    pub fn query_field(&self) -> &Entity<Editor> {
        &self.query
    }

    pub fn replace_field(&self) -> &Entity<Editor> {
        &self.replace
    }

    pub fn mask_field(&self) -> &Entity<Editor> {
        &self.mask
    }

    pub fn directory_field(&self) -> &Entity<Editor> {
        &self.directory
    }

    pub fn preview_editor(&self) -> &Entity<Editor> {
        self.preview.editor()
    }

    pub fn results(&self) -> &Results {
        &self.results
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn is_searching(&self) -> bool {
        self.status == Status::Searching
    }

    /// A search runs (not only waits for the typing to pause): the loading
    /// indicator shows, as the reference's does once its search starts.
    fn is_running(&self) -> bool {
        self.status == Status::Searching && self.search_task.is_some()
    }

    /// The counts of the header with `+` while there may be more:
    /// matches and files.
    fn counts(&self) -> (String, String) {
        let (m, f) = (self.results.matches(), self.results.files());
        let plus = if self.loading_more || (self.truncated && self.status == Status::Done) {
            "+"
        } else {
            ""
        };
        (format!("{m}{plus}"), format!("{f}{plus}"))
    }

    /// The counter of the header: `3 matches in 2 files`, with `+` when the
    /// search stopped at its limit; empty without matches.
    pub fn info(&self) -> String {
        let (m, f) = (self.results.matches(), self.results.files());
        if m == 0 || self.stale {
            return String::new();
        }
        let (mc, fc) = self.counts();
        let matches = if m == 1 { "match" } else { "matches" };
        let files = if f == 1 { "file" } else { "files" };
        format!("{mc} {matches} in {fc} {files}")
    }

    /// How long the last search took to its first rows and to its end
    /// (`None` before it started).
    pub fn timing(&self) -> Option<(Option<Duration>, Option<Duration>)> {
        self.started?;
        Some((self.first_rows, self.finished))
    }

    /// The text shown in place of the rows when there are none.
    pub fn empty_text(&self) -> Option<String> {
        if !self.results.is_empty() && !self.stale {
            return None;
        }
        Some(match &self.status {
            Status::Empty => "Type search query to find in files".into(),
            Status::Searching => "Searching…".into(),
            Status::Done => "Nothing found".into(),
            Status::Invalid(message) => message.clone(),
            Status::NoProject => "No project is open".into(),
        })
    }

    /// The named scopes that have files, in their order.
    fn named_scopes(&self) -> Vec<NamedScope> {
        named_scopes(&self.context)
    }

    /// Files shown in editors and the open files, most recent first.
    fn recently_viewed(&self) -> Vec<PathBuf> {
        recently_viewed(&self.context)
    }

    fn rel(&self, p: &Path) -> String {
        self.context
            .root
            .as_ref()
            .and_then(|r| p.strip_prefix(r).ok())
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// The modules of the Module tab: the project.
    fn module_name(&self) -> String {
        self.context
            .root
            .as_ref()
            .and_then(|r| r.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let rows: Vec<_> = if self.stale {
            Vec::new()
        } else {
            self.results
                .rows()
                .iter()
                .map(|r| {
                    json!({
                        "path": self.rel(&r.path),
                        "line": r.line.line + 1,
                        "text": r.line.text,
                        "ranges": r.line.ranges.iter().map(|m| [m.start, m.end]).collect::<Vec<_>>(),
                    })
                })
                .collect()
        };
        let scope = match self.scope {
            ScopeKind::Project => "project".to_owned(),
            ScopeKind::Module => "module".to_owned(),
            ScopeKind::Directory => "directory".to_owned(),
            ScopeKind::Named => self.named.name().to_owned(),
        };
        json!({
            "open": self.open,
            "replace_mode": self.replace_mode,
            "text": self.query.read(cx).text(),
            "replace": self.replace.read(cx).text(),
            "case_sensitive": self.options.case_sensitive,
            "whole_words": self.options.whole_words,
            "regex": self.options.regex,
            "mask": { "on": self.mask_on, "text": self.mask.read(cx).text() },
            "scope": scope,
            "module": self.module_name(),
            "directory": self.directory.read(cx).text(),
            "recursive": self.recursive,
            "scopes": self.named_scopes().iter().map(|s| s.name()).collect::<Vec<_>>(),
            "pinned": self.pinned,
            "searching": self.is_searching(),
            "info": self.info(),
            "empty_text": self.empty_text(),
            "matches": if self.stale { 0 } else { self.results.matches() },
            "files": if self.stale { 0 } else { self.results.files() },
            "truncated": self.truncated,
            "large_files": self.large_files,
            "held_back": self.held_back,
            "rows": rows,
            "selected": self.selected,
            "loading_more": self.loading_more,
            "scroll_top": f32::from(-self.list.0.borrow().base_handle.offset().y),
            "dropdown": self.dropdown.as_ref().map(|d| d.state_json()),
            "preview": self.preview.state_json(self, cx),
            "split": self.split,
            "dragging": {
                "popup": self.frame.is_dragging(),
                "split": self.split_drag.is_dragging(),
            },
            "browsing": self.browsing,
            "browse_requests": self.browse_requests,
        })
    }

    // ---- opening and closing ----------------------------------------------

    /// Opens the popup (or refreshes it when open): the selected text, else
    /// the search of an open find bar, else the last search fills the field,
    /// selected; a folder selected in the project tool window becomes the
    /// Directory scope. The search starts at once.
    pub fn open(
        &mut self,
        replace: bool,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            self.take_shared(cx);
        }
        self.show(replace, context, window, cx);
    }

    /// The options and the mask are shared with Search Everywhere's Text
    /// tab: takes them as they are now.
    fn take_shared(&mut self, cx: &mut Context<Self>) {
        let shared = settings(cx);
        self.options = shared.options();
        self.mask_on = shared.file_mask.is_some();
        let mask = shared.shown_mask();
        set_field_text(&self.mask, &mask, cx);
    }

    fn show(
        &mut self,
        replace: bool,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut text = self.query.read(cx).text();
        if let Some(selected) = context
            .selection
            .clone()
            .filter(|s| !s.is_empty() && s.len() < 10_000)
        {
            text = if self.options.regex {
                search::escape(&selected)
            } else {
                selected
            };
        } else if let Some(bar) = context.find_bar_text.clone() {
            text = bar;
        } else if text.is_empty() {
            text = editor::find::last_search(cx)
                .history
                .first()
                .cloned()
                .unwrap_or_default();
        }
        if let Some(folder) = &context.tree_folder {
            self.scope = ScopeKind::Directory;
            let folder = folder.display().to_string();
            set_field_text(&self.directory, &folder, cx);
        } else if self.directory.read(cx).text().is_empty()
            && let Some(dir) = context
                .current_file
                .as_ref()
                .and_then(|f| f.parent())
                .or(context.root.as_deref())
        {
            let dir = dir.display().to_string();
            set_field_text(&self.directory, &dir, cx);
        }
        if !named_scopes(&context).contains(&self.named) {
            self.named = NamedScope::ProjectFiles;
        }
        self.context = context;
        if !self.open || self.replace_mode != replace {
            self.results = self.empty_results_for(replace);
            self.forget_marks();
            self.stale = false;
            self.selected = None;
        }
        self.replace_mode = replace;
        self.open = true;
        self.dropdown = None;
        self.query
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        if replace {
            let last = editor::find::last_search(cx).replace;
            if self.replace.read(cx).text().is_empty() {
                set_field_text(&self.replace, &last, cx);
            }
        }
        window.focus(&self.query.focus_handle(cx));
        self.search_now(cx);
        cx.notify();
    }

    /// Opens the popup and searches for `text` with the given options and
    /// scope.
    pub fn search_for(
        &mut self,
        args: &Search,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            self.take_shared(cx);
        }
        let options = SearchOptions {
            case_sensitive: args.case_sensitive.unwrap_or(self.options.case_sensitive),
            whole_words: args.whole_words.unwrap_or(self.options.whole_words),
            regex: args.regex.unwrap_or(self.options.regex),
        };
        self.set_options(options, cx);
        if let Some(mask) = &args.mask {
            self.mask_on = !mask.is_empty();
            if !mask.is_empty() {
                set_field_text(&self.mask, mask, cx);
            }
        }
        if let Some(recursive) = args.recursive {
            self.set_recursive(recursive, cx);
        }
        if let Some(dir) = &args.directory {
            let dir = match &context.root {
                Some(root) if Path::new(dir).is_relative() => {
                    native_path::join(root, Path::new(dir))
                        .display()
                        .to_string()
                }
                _ => dir.clone(),
            };
            set_field_text(&self.directory, &dir, cx);
        }
        let replace = args.replace.is_some() || (self.open && self.replace_mode);
        self.show(replace, context, window, cx);
        match args.scope.as_deref() {
            Some("project") => self.scope = ScopeKind::Project,
            Some("module") => self.scope = ScopeKind::Module,
            Some("directory") => self.scope = ScopeKind::Directory,
            Some(name) => {
                if let Some(named) = NamedScope::from_name(name) {
                    self.scope = ScopeKind::Named;
                    self.named = named;
                }
            }
            None => {}
        }
        if let Some(text) = &args.replace {
            set_field_text(&self.replace, text, cx);
        }
        self.query
            .update(cx, |f, cx| f.set_text_selected(&args.text, cx));
        window.focus(&self.query.focus_handle(cx));
        self.search_now(cx);
    }

    /// Closes the popup; with `restore_focus` (Escape) the focus goes back
    /// to where it was. The search is stopped, the settings are saved.
    pub fn close(&mut self, restore_focus: bool, _: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.dropdown = None;
        self.frame.cancel_drag();
        self.split_drag.end();
        self.cancel_search();
        self.clear_preview(cx);
        self.save(cx);
        if restore_focus {
            cx.emit(FindInFilesEvent::Dismissed);
        }
        cx.notify();
    }

    /// Remembers the search and the settings (the reference saves them when
    /// the popup closes or a result is opened).
    fn save(&mut self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).text();
        editor::find::remember_search(cx, &text);
        if self.replace_mode {
            let replace = self.replace.read(cx).text();
            editor::find::remember_replace(cx, &replace);
        }
        let mask = self.mask.read(cx).text();
        let directory = self.directory.read(cx).text();
        let (mask_on, options, pinned) = (self.mask_on, self.options, self.pinned);
        let place = self.frame.place();
        let (scope, named, split, recursive) = (self.scope, self.named, self.split, self.recursive);
        update_settings(cx, |s| {
            if place.is_some() {
                s.popup = place;
            }
            s.file_mask = mask_on.then(|| mask.clone());
            if mask_on {
                s.remember_mask(&mask);
            }
            if scope == ScopeKind::Directory {
                s.remember_directory(&directory);
            }
            if scope == ScopeKind::Named {
                s.search_scope = Some(named.name().to_owned());
            }
            s.case_sensitive = options.case_sensitive;
            s.whole_words = options.whole_words;
            s.regex = options.regex;
            s.pinned = pinned;
            s.recursive = recursive;
            s.splitter = Some(split);
        });
    }

    // ---- searching --------------------------------------------------------

    fn set_options(&mut self, options: SearchOptions, cx: &mut Context<Self>) {
        self.options = options;
        update_settings(cx, |s| {
            s.case_sensitive = options.case_sensitive;
            s.whole_words = options.whole_words;
            s.regex = options.regex;
        });
    }

    fn set_recursive(&mut self, recursive: bool, cx: &mut Context<Self>) {
        self.recursive = recursive;
        update_settings(cx, |s| s.recursive = recursive);
    }

    fn cancel_search(&mut self) {
        self.debounce = None;
        self.search_task = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    /// Searches again after [`DEBOUNCE`] unless another change comes first.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.cancel_search();
        self.status = Status::Searching;
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.search_now(cx));
        }));
        cx.notify();
    }

    /// The files of the named scope.
    fn named_files(&self) -> Vec<PathBuf> {
        match self.named {
            NamedScope::ProjectFiles => Vec::new(),
            NamedScope::RecentlyViewed => self.recently_viewed(),
            NamedScope::RecentlyChanged => self.context.recently_changed.clone(),
            NamedScope::OpenFiles => self
                .context
                .editors
                .iter()
                .map(|(p, _)| p.clone())
                .collect(),
            NamedScope::CurrentFile => self.context.current_file.iter().cloned().collect(),
        }
    }

    /// Open documents with unsaved changes, searched instead of their files.
    fn overrides(&self, cx: &App) -> HashMap<PathBuf, Rope> {
        let mut overrides: HashMap<PathBuf, Rope> = self
            .context
            .editors
            .iter()
            .filter(|(_, e)| e.read(cx).is_modified())
            .map(|(p, e)| (p.clone(), e.read(cx).buffer().rope().clone()))
            .collect();
        // A file edited in the preview and not saved yet.
        if let Some(path) = self.preview.path(cx)
            && let Some(editor) = self.preview_own_editor(path, cx)
            && editor.read(cx).is_modified()
        {
            overrides.insert(path.clone(), editor.read(cx).buffer().rope().clone());
        }
        overrides
    }

    /// What is searched for, or why nothing is (with the field at fault).
    fn build_search(&self, cx: &App) -> Result<FileSearch, (Status, Option<Field>)> {
        let text = self.query.read(cx).text();
        let invalid = |m: String, f: Field| (Status::Invalid(m), Some(f));
        let mask = if self.mask_on {
            let mask = self.mask.read(cx).text();
            if mask.trim().is_empty() {
                return Err(invalid("Empty file mask".into(), Field::Mask));
            }
            if mask.contains(';') {
                return Err(invalid(
                    "File masks should be comma-separated".into(),
                    Field::Mask,
                ));
            }
            Some(
                FileMask::parse(&mask)
                    .map_err(|_| invalid(format!("Bad file mask \"{mask}\""), Field::Mask))?,
            )
        } else {
            None
        };
        let matcher = match Query::new(&text, self.options).compile() {
            Ok(m) => m,
            // Nothing to search for: no results, also with a file mask.
            Err(QueryError::Empty) => return Err((Status::Empty, None)),
            Err(QueryError::Vague) => {
                return Err(invalid(
                    "Regular expression matches empty string".into(),
                    Field::Query,
                ));
            }
            Err(QueryError::InvalidRegex(message)) => {
                return Err(invalid(
                    format!("Bad pattern \"{text}\": {message}"),
                    Field::Query,
                ));
            }
        };
        if self.options.regex && matches_empty(&text) {
            return Err(invalid(
                "Regular expression matches empty string".into(),
                Field::Query,
            ));
        }
        let root = self.context.root.clone().ok_or((Status::NoProject, None))?;
        let scope = match self.scope {
            ScopeKind::Project | ScopeKind::Module => Scope::Directory {
                path: root.clone(),
                recursive: true,
            },
            ScopeKind::Directory => {
                let dir = self.directory.read(cx).text();
                // A relative folder is one of the project's.
                let path = native_path::join(&root, Path::new(dir.trim()));
                if dir.trim().is_empty() || !path.is_dir() {
                    return Err(invalid("Directory is not found".into(), Field::Directory));
                }
                Scope::Directory {
                    path,
                    recursive: self.recursive,
                }
            }
            ScopeKind::Named => match self.named {
                NamedScope::ProjectFiles => Scope::Directory {
                    path: root.clone(),
                    recursive: true,
                },
                _ => Scope::Files(self.named_files()),
            },
        };
        Ok(FileSearch {
            matcher,
            mask,
            scope,
            root,
            filter: self.context.filter.clone(),
            limit: PAGE_SIZE,
            known: Default::default(),
            overrides: self.overrides(cx),
            project_files: false,
        })
    }

    /// Starts a search now, cancelling the running one.
    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        self.generation += 1;
        for field in [&self.query, &self.mask, &self.directory] {
            field.update(cx, |f, cx| f.set_error(false, cx));
        }
        let search = match self.build_search(cx) {
            Ok(search) => search,
            Err((status, field)) => {
                if let Some(field) = field {
                    self.field(field).update(cx, |f, cx| f.set_error(true, cx));
                }
                self.invalid_field = field;
                self.status = status;
                self.results = self.empty_results();
                self.forget_marks();
                self.stale = false;
                self.selected = None;
                self.truncated = false;
                self.last_search = None;
                self.clear_preview(cx);
                cx.notify();
                return;
            }
        };
        self.invalid_field = None;
        self.stale = true;
        self.truncated = false;
        self.loading_more = false;
        self.last_search = Some(search.clone());
        self.started = Some(std::time::Instant::now());
        self.first_rows = None;
        self.finished = None;
        self.run_pass(search, cx);
    }

    fn field(&self, field: Field) -> &Entity<Editor> {
        match field {
            Field::Query => &self.query,
            Field::Replace => &self.replace,
            Field::Mask => &self.mask,
            Field::Directory => &self.directory,
        }
    }

    /// Loads more results (the user scrolled to the end, or all rows fit):
    /// the same search again for [`PAGE_SIZE`]` / 2` more matches, the rows
    /// on screen staying where they are.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(mut search) = self.last_search.clone() else {
            return;
        };
        if self.status != Status::Done || !self.truncated || self.stale || self.loading_more {
            return;
        }
        self.generation += 1;
        self.results.freeze();
        search.limit = PAGE_SIZE / 2;
        search.known = std::sync::Arc::new(self.results.lines_by_file());
        self.loading_more = true;
        self.run_pass(search, cx);
    }

    /// Whether more results should load: the last pass stopped at its
    /// limit and the list is scrolled near its end, or shows every row.
    fn wants_more(&self) -> bool {
        if self.status != Status::Done || !self.truncated || self.stale || self.loading_more {
            return false;
        }
        let list = self.list.0.borrow();
        let handle = &list.base_handle;
        let extent = handle.max_offset().height;
        let rest = extent + handle.offset().y;
        let viewport = handle.bounds().size.height;
        extent <= px(0.0) || rest < (viewport / 2.0).max(px(50.0))
    }

    /// A list for new results (a row per match in replace mode).
    fn empty_results(&self) -> Results {
        self.empty_results_for(self.replace_mode)
    }

    fn empty_results_for(&self, replace: bool) -> Results {
        if replace {
            Results::per_match()
        } else {
            Results::new()
        }
    }

    /// Runs one pass of a search in the background, streaming its files.
    fn run_pass(&mut self, search: FileSearch, cx: &mut Context<Self>) {
        self.status = Status::Searching;
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let run = cx.background_spawn(async move {
            search.run(&cancel, &move |found| {
                let _ = tx.unbounded_send(found);
            })
        });
        let generation = self.generation;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                if this
                    .update(cx, |this, cx| this.found(generation, batch, cx))
                    .is_err()
                {
                    return;
                }
            }
            let summary = run.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.found(generation, Vec::new(), cx);
                this.status = Status::Done;
                if !this.loading_more {
                    this.finished = this.started.map(|t| t.elapsed());
                }
                this.truncated = summary.truncated;
                this.loading_more = false;
                this.large_files = summary.large_files.len();
                this.held_back = summary.held_back;
                this.cancel = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Files with matches came in: the rows of the previous search go, the
    /// new ones take their places; the selected row stays selected.
    fn found(
        &mut self,
        generation: u64,
        files: Vec<project_search::FileMatches>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        if self.stale {
            self.results = self.empty_results();
            self.forget_marks();
            self.stale = false;
            self.selected = None;
        }
        let selected = self.selected.and_then(|i| self.results.row(i)).map(|r| {
            let column = r.line.ranges.first().map_or(0, |m| m.start);
            (r.path.clone(), r.line.line, column)
        });
        if !files.is_empty() && self.first_rows.is_none() {
            self.first_rows = self.started.map(|t| t.elapsed());
        }
        for file in files {
            self.results.add(file);
        }
        self.sync_marks(cx);
        let before = self.selected;
        let had_selection = selected.is_some();
        self.selected = match selected {
            Some((path, line, column)) => self.results.position(&path, line, column),
            None if !self.results.is_empty() => {
                self.list.scroll_to_item(0, ScrollStrategy::Top);
                Some(0)
            }
            None => None,
        };
        if self.selected != before || !had_selection {
            self.preview_selected(cx);
        }
        cx.notify();
    }

    // ---- moving and opening -----------------------------------------------

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.stale || self.results.is_empty() {
            return;
        }
        let index = index.min(self.results.len() - 1);
        self.selected = Some(index);
        // As little scrolling as shows the row: a row above the view comes
        // in at the top, one below it at the bottom.
        let row_top = cx.theme().metric(Metric::RowHeight) * index as f32;
        let scroll_top = -self.list.0.borrow().base_handle.offset().y;
        let strategy = if row_top < scroll_top {
            ScrollStrategy::Top
        } else {
            ScrollStrategy::Bottom
        };
        self.list.scroll_to_item(index, strategy);
        self.preview_selected(cx);
        cx.notify();
    }

    /// Moves the selection by `delta` rows (not past the ends).
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(current) = self.selected else {
            self.select(0, cx);
            return;
        };
        let next = current.saturating_add_signed(delta);
        self.select(next, cx);
    }

    /// Up and Down in the search field: the rows, or, while the field holds
    /// several lines, the caret (the key goes on to the field's binding).
    fn up_down(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        if self.focused_field(window, cx).read(cx).buffer().len_lines() > 1 {
            cx.propagate();
            return;
        }
        self.move_selection(delta, cx);
    }

    /// The field with the focus (the search field when none has).
    fn focused_field(&self, window: &Window, cx: &App) -> &Entity<Editor> {
        [&self.replace, &self.mask, &self.directory]
            .into_iter()
            .find(|f| f.focus_handle(cx).is_focused(window))
            .unwrap_or(&self.query)
    }

    fn focused_field_kind(&self, window: &Window, cx: &App) -> Field {
        [
            (Field::Replace, &self.replace),
            (Field::Mask, &self.mask),
            (Field::Directory, &self.directory),
        ]
        .into_iter()
        .find(|(_, f)| f.focus_handle(cx).is_focused(window))
        .map_or(Field::Query, |(k, _)| k)
    }

    /// Opens the selected row's file at its match; the popup closes unless
    /// it is pinned.
    pub fn open_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected
            .filter(|_| !self.stale)
            .and_then(|i| self.results.row(i))
        else {
            return;
        };
        let path = (*row.path).clone();
        // Where the match is now (edits in the preview move it).
        let point = self.row_point(row);
        self.open_at(path, point, window, cx);
    }

    /// Opens a file at a point and closes the popup unless it is pinned.
    fn open_at(
        &mut self,
        path: PathBuf,
        point: Point,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A document edited in the preview goes to the tab (the popup does
        // not write it as it closes).
        let document = if self.pinned {
            self.preview_document(&path, cx)
        } else {
            self.take_preview_document(&path, cx)
        };
        self.save(cx);
        if !self.pinned {
            self.close(false, window, cx);
        }
        cx.emit(FindInFilesEvent::OpenAt {
            path,
            point,
            document,
        });
    }

    fn toggle_mask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mask_on = !self.mask_on;
        let field = if self.mask_on {
            &self.mask
        } else {
            &self.query
        };
        if self.mask_on {
            self.mask.update(cx, |f, cx| {
                let text = f.text();
                f.set_text_selected(&text, cx)
            });
        }
        window.focus(&field.focus_handle(cx));
        self.schedule(cx);
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.pinned = !self.pinned;
        let pinned = self.pinned;
        update_settings(cx, |s| s.pinned = pinned);
        cx.notify();
    }

    fn toggle_option(&mut self, f: impl FnOnce(&mut SearchOptions), cx: &mut Context<Self>) {
        let mut options = self.options;
        f(&mut options);
        self.set_options(options, cx);
        self.schedule(cx);
    }

    fn set_scope(&mut self, scope: ScopeKind, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = scope;
        self.dropdown = None;
        if scope == ScopeKind::Directory {
            self.directory.update(cx, |f, cx| {
                let text = f.text();
                f.set_text_selected(&text, cx)
            });
            window.focus(&self.directory.focus_handle(cx));
        } else if self.directory.focus_handle(cx).is_focused(window) {
            window.focus(&self.query.focus_handle(cx));
        }
        self.schedule(cx);
    }

    /// Shows a folder chooser for the Directory scope.
    fn browse_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browse_requests += 1;
        if !crate::shot::native_windows() {
            cx.notify();
            return;
        }
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        self.browsing = true;
        cx.spawn_in(window, async move |this, cx| {
            let picked = chosen.await.ok().and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                this.browsing = false;
                if let Some(path) = picked.and_then(|p| p.into_iter().next()) {
                    let text = path.display().to_string();
                    set_field_text(&this.directory, &text, cx);
                    this.scope = ScopeKind::Directory;
                    this.schedule(cx);
                }
                window.focus(&this.directory.focus_handle(cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The fields Tab goes through, in order.
    fn tab_order(&self) -> Vec<Field> {
        let mut order = vec![Field::Query];
        if self.replace_mode {
            order.push(Field::Replace);
        }
        if self.mask_on {
            order.push(Field::Mask);
        }
        if self.scope == ScopeKind::Directory {
            order.push(Field::Directory);
        }
        order
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.tab_order();
        let current = self.focused_field_kind(window, cx);
        let i = order.iter().position(|f| *f == current).unwrap_or(0);
        let next = if forward {
            (i + 1) % order.len()
        } else {
            (i + order.len() - 1) % order.len()
        };
        let field = self.field(order[next]).clone();
        field.update(cx, |f, cx| {
            let text = f.text();
            f.set_text_selected(&text, cx)
        });
        window.focus(&field.focus_handle(cx));
        cx.notify();
    }

    // ---- actions ----------------------------------------------------------

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        self.close(true, window, cx);
    }

    /// The next row; in the preview Find Next, as in any editor (the
    /// reference binds its row keys to the fields and the list only).
    fn next_result_action(&mut self, _: &NextResult, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_preview(window, cx) {
            self.find_in_preview(editor::find::Direction::Forward, window, cx);
            return;
        }
        self.move_selection(1, cx);
    }

    fn previous_result_action(
        &mut self,
        _: &PreviousResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            self.find_in_preview(editor::find::Direction::Backward, window, cx);
            return;
        }
        self.move_selection(-1, cx);
    }

    /// The editor's Find Next (F3, Ctrl+L): the next match in the preview;
    /// in the fields the next row, as the reference has it.
    fn find_next_action(
        &mut self,
        _: &editor::find::Next,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            self.find_in_preview(editor::find::Direction::Forward, window, cx);
            return;
        }
        self.move_selection(1, cx);
    }

    fn find_previous_action(
        &mut self,
        _: &editor::find::Previous,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            self.find_in_preview(editor::find::Direction::Backward, window, cx);
            return;
        }
        self.move_selection(-1, cx);
    }

    /// Edit Source (F4): from the preview the file at the preview's caret,
    /// else the selected row.
    fn edit_source_action(&mut self, _: &EditSource, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_preview(window, cx)
            && let Some(path) = self.preview.path(cx).cloned()
        {
            let point = self.preview.editor().read(cx).caret_point();
            self.open_at(path, point, window, cx);
            return;
        }
        self.open_result(window, cx);
    }

    fn down_action(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.up_down(1, window, cx);
    }

    fn up_action(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.up_down(-1, window, cx);
    }

    fn page_down_action(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(PAGE_ROWS as isize, cx);
    }

    fn page_up_action(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-(PAGE_ROWS as isize), cx);
    }

    /// The preview has the focus: Enter, Shift+Enter, Tab and Option+Down
    /// (Alt+Down) are the editor's there, as in the reference IDE (its
    /// popup actions are off while the focus is in an editor).
    fn in_preview(&self, window: &Window, cx: &App) -> bool {
        self.preview.editor().focus_handle(cx).is_focused(window)
    }

    fn open_result_action(&mut self, _: &OpenResult, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_preview(window, cx) {
            cx.propagate();
            return;
        }
        self.open_result(window, cx);
    }

    fn replace_selected_action(
        &mut self,
        _: &ReplaceSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            cx.propagate();
            return;
        }
        self.replace_selected(window, cx);
    }

    fn replace_all_action(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_all(window, cx);
    }

    fn toggle_mask_action(&mut self, _: &ToggleMask, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mask(window, cx);
    }

    fn toggle_pin_action(&mut self, _: &TogglePin, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_pin(cx);
    }

    fn scope_project_action(
        &mut self,
        _: &ScopeProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(ScopeKind::Project, window, cx);
    }

    fn scope_module_action(
        &mut self,
        _: &ScopeModule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(ScopeKind::Module, window, cx);
    }

    fn scope_directory_action(
        &mut self,
        _: &ScopeDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(ScopeKind::Directory, window, cx);
    }

    fn scope_named_action(&mut self, _: &ScopeNamed, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scope(ScopeKind::Named, window, cx);
    }

    fn toggle_recursive_action(
        &mut self,
        _: &ToggleRecursive,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let recursive = !self.recursive;
        self.set_recursive(recursive, cx);
        if self.scope == ScopeKind::Directory {
            self.schedule(cx);
        }
        cx.notify();
    }

    fn browse_directory_action(
        &mut self,
        _: &BrowseDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browse_directory(window, cx);
    }

    fn focus_preview_action(
        &mut self,
        _: &FocusPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview.path(cx).is_some() {
            window.focus(&self.preview.editor().focus_handle(cx));
        }
    }

    fn next_field_action(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_preview(window, cx) {
            cx.propagate();
            return;
        }
        self.move_focus(true, window, cx);
    }

    fn previous_field_action(
        &mut self,
        _: &PreviousField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            cx.propagate();
            return;
        }
        self.move_focus(false, window, cx);
    }

    fn toggle_case_action(
        &mut self,
        _: &editor::find::ToggleCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_option(|o| o.case_sensitive = !o.case_sensitive, cx);
    }

    fn toggle_words_action(
        &mut self,
        _: &editor::find::ToggleWords,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_option(|o| o.whole_words = !o.whole_words, cx);
    }

    fn toggle_regex_action(
        &mut self,
        _: &editor::find::ToggleRegex,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_option(|o| o.regex = !o.regex, cx);
    }

    fn new_line_action(
        &mut self,
        _: &editor::find::NewLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = match self.focused_field_kind(window, cx) {
            Field::Replace => self.replace.clone(),
            _ => self.query.clone(),
        };
        window.focus(&field.focus_handle(cx));
        field.update(cx, |f, cx| f.insert("\n", cx));
    }
}

/// Puts text in a field with the caret at its end: a field without the
/// focus shows no selection.
fn set_field_text(field: &Entity<Editor>, text: &str, cx: &mut App) {
    field.update(cx, |f, cx| {
        f.set_text_selected(text, cx);
        let end = f.buffer().len_chars();
        f.set_selections(text::SelectionSet::caret(end), cx);
    });
}

/// The named scopes that have files in a context, in their order.
fn named_scopes(context: &OpenContext) -> Vec<NamedScope> {
    NamedScope::ALL
        .into_iter()
        .filter(|s| match s {
            NamedScope::ProjectFiles => true,
            NamedScope::RecentlyViewed => !recently_viewed(context).is_empty(),
            NamedScope::RecentlyChanged => !context.recently_changed.is_empty(),
            NamedScope::OpenFiles => !context.editors.is_empty(),
            NamedScope::CurrentFile => context.current_file.is_some(),
        })
        .collect()
}

/// Files shown in editors and then the other open files, most recent
/// first, as the reference's Recently Viewed Files.
fn recently_viewed(context: &OpenContext) -> Vec<PathBuf> {
    let mut files = context.recently_viewed.clone();
    for (path, _) in &context.editors {
        if !files.contains(path) {
            files.push(path.clone());
        }
    }
    files.truncate(RECENT_FILES);
    files
}

/// Whether a regular expression matches the empty string (and is not
/// anchored), which the reference does not search for.
fn matches_empty(pattern: &str) -> bool {
    if pattern.starts_with('^') || pattern.ends_with('$') {
        return false;
    }
    regex::Regex::new(&format!("^(?:{pattern})$")).is_ok_and(|r| r.is_match(""))
}

macro_rules! popup_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl FindInFiles {
            fn bind_actions<E: InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }
    };
}

popup_actions! {
    Close => close_action,
    NextResult => next_result_action,
    PreviousResult => previous_result_action,
    Down => down_action,
    Up => up_action,
    PageDown => page_down_action,
    PageUp => page_up_action,
    OpenResult => open_result_action,
    ReplaceSelected => replace_selected_action,
    ReplaceAll => replace_all_action,
    ToggleMask => toggle_mask_action,
    TogglePin => toggle_pin_action,
    ScopeProject => scope_project_action,
    ScopeModule => scope_module_action,
    ScopeDirectory => scope_directory_action,
    ScopeNamed => scope_named_action,
    ToggleRecursive => toggle_recursive_action,
    BrowseDirectory => browse_directory_action,
    NextField => next_field_action,
    FocusPreview => focus_preview_action,
    EditSource => edit_source_action,
    editor::find::Next => find_next_action,
    editor::find::Previous => find_previous_action,
    PreviousField => previous_field_action,
    editor::find::ToggleCase => toggle_case_action,
    editor::find::ToggleWords => toggle_words_action,
    editor::find::ToggleRegex => toggle_regex_action,
    editor::find::NewLine => new_line_action,
    editor::find::ShowHistory => show_dropdown_action,
    editor::find::HistoryUp => dropdown_up_action,
    editor::find::HistoryDown => dropdown_down_action,
    editor::find::HistoryPick => dropdown_pick_action,
    editor::find::HistoryClose => dropdown_close_action,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regexes_that_match_nothing_but_empty_text() {
        assert!(matches_empty("a*"));
        assert!(matches_empty("(x)?"));
        assert!(!matches_empty("a+"));
        assert!(!matches_empty("^"));
        assert!(!matches_empty("foo$"));
        assert!(!matches_empty("("));
    }

    #[test]
    fn named_scopes_by_name() {
        for s in NamedScope::ALL {
            assert_eq!(NamedScope::from_name(s.name()), Some(s));
        }
        assert_eq!(NamedScope::from_name("All Places"), None);
    }
}
