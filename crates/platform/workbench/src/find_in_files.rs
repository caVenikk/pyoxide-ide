//! Find in Files: a movable popup over the main window with a search field
//! (Match Case, Words, Regex), a file mask, the scope and the matching lines
//! of the project's files, as in the reference IDE
//! (tests/scenarios/editor/find-in-files.md).
//!
//! The search runs in the background (`project-search`): every change of
//! the query, the options or the mask waits [`DEBOUNCE`] for the next one,
//! then cancels the running search and starts a new one. Files with matches
//! stream in and are shown at once, ordered by [`Results`]; the rows of the
//! previous search stay until the first new row arrives. Like the
//! reference, a search stops after [`PAGE_SIZE`] matches and the counter
//! then reads `100+ matches in 12+ files`.
//!
//! Enter opens the selected line in an editor at its match and closes the
//! popup unless it is pinned; Up and Down move through the rows while the
//! focus stays in the field. The popup closes when the focus moves
//! elsewhere in the window, unless it is pinned. The file mask, its check
//! box, the options and the pin are remembered across sessions
//! (`find.toml` in the configuration folder), as the reference remembers
//! them in its application settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use editor::Editor;
use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Global, HighlightStyle, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render, ScrollStrategy,
    SharedString, StyledText, Subscription, Task, UniformListScrollHandle, Window, canvas, div,
    point, prelude::*, px, uniform_list,
};
use project_search::{FileMask, FileSearch, Results, Scope};
use search::{CancelToken, Query, QueryError, SearchOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;
use text::{Buffer, Point, Rope};
use themes::{Metric, UiRole};
use ui::components::{
    SMALL_BUTTON, Tooltip, checkbox, input_frame, small_icon_button, toggle_button,
};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the Find in Files popup.
    Open => "find-in-files.open", "Find in Files…",
        "Opens the Find in Files popup with the selected text, or the last search, selected in its field and searches the project.";
    /// Search the project for a text.
    Search {
        text: String,
        case_sensitive: Option<bool>,
        whole_words: Option<bool>,
        regex: Option<bool>,
        mask: Option<String>,
    } => "find-in-files.search", "Find in Files for Text",
        "Opens the Find in Files popup and searches the project for a text; options that are given are switched on or off, a mask narrows the files (an empty mask switches it off).",
        r#"{"text": "def", "case_sensitive": false, "whole_words": false, "regex": false, "mask": "*.py"}"#;
    /// Describe the popup.
    State => "find-in-files.state", "Find in Files State",
        "Returns the state of the Find in Files popup as JSON (query, options, mask, status, rows, selected row).";
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
    /// File mask on or off.
    ToggleMask => "find-in-files.toggle-mask", "File Mask",
        "Switches the file mask on (and puts the focus in it) or off.";
    /// Keep the popup open.
    TogglePin => "find-in-files.toggle-pin", "Pin Window",
        "Keeps the popup open when the focus moves elsewhere or a result is opened.";
}

/// Matches one search collects before it stops (the reference IDE's
/// `ide.usages.page.size`).
pub const PAGE_SIZE: usize = 100;
/// Wait after a change before searching again (the reference's delay).
pub const DEBOUNCE: Duration = Duration::from_millis(100);
/// Most recent file masks kept.
pub const RECENT_MASKS: usize = 300;
/// Distance of the popup from the top of the window, as in the reference.
const TOP: f32 = 115.0;
/// Default size of the popup, and the room kept around it.
const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 620.0;
const MARGIN: f32 = 16.0;
const HEADER_HEIGHT: f32 = 40.0;
const SCOPE_HEIGHT: f32 = 36.0;
/// Height of the search field for one line.
const FIELD_HEIGHT: f32 = 36.0;
const PAD: f32 = 12.0;
/// Width of the file mask field.
const MASK_WIDTH: f32 = 140.0;
/// Rows a page up or down moves.
const PAGE_ROWS: usize = 10;

/// The Find in Files settings that last across sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FindInFilesSettings {
    /// The mask in use while the File mask box is checked.
    pub file_mask: Option<String>,
    /// Masks used before, newest first.
    pub recent_masks: Vec<String>,
    pub case_sensitive: bool,
    pub whole_words: bool,
    pub regex: bool,
    /// The popup stays open when it loses the focus.
    pub pinned: bool,
}

impl FindInFilesSettings {
    /// Settings before any were saved: the product's masks to offer.
    pub fn with_masks(masks: &[&str]) -> Self {
        Self {
            recent_masks: masks.iter().map(|m| (*m).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            regex: self.regex,
        }
    }

    /// The mask shown in the field: the one in use, else the most recent.
    fn shown_mask(&self) -> String {
        self.file_mask
            .clone()
            .or_else(|| self.recent_masks.first().cloned())
            .unwrap_or_default()
    }

    fn remember_mask(&mut self, mask: &str) {
        if mask.is_empty() {
            return;
        }
        self.recent_masks.retain(|m| m != mask);
        self.recent_masks.insert(0, mask.to_owned());
        self.recent_masks.truncate(RECENT_MASKS);
    }
}

/// The settings and the file they are kept in (none in tests).
#[derive(Default)]
struct SettingsStore {
    settings: FindInFilesSettings,
    file: Option<PathBuf>,
}

impl Global for SettingsStore {}

/// Registers the commands and the default settings (the product's masks).
pub fn init(cx: &mut App, masks: &[&str]) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
    cx.set_global(SettingsStore {
        settings: FindInFilesSettings::with_masks(masks),
        file: None,
    });
}

/// Where the settings are kept: `PYOXIDE_CONFIG_DIR`, else the
/// configuration folder of the user, in `pyoxide/find.toml`.
pub fn settings_file() -> Option<PathBuf> {
    let dir = match std::env::var_os("PYOXIDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::config_dir()?.join("pyoxide"),
    };
    Some(dir.join("find.toml"))
}

/// Reads the settings from `file` (keeping the defaults when there is none)
/// and saves them there from now on.
pub fn load_settings(cx: &mut App, file: PathBuf) {
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<FindInFilesSettings>(&text).ok());
    let store = cx.default_global::<SettingsStore>();
    if let Some(settings) = loaded {
        store.settings = settings;
    }
    store.file = Some(file);
}

/// The current settings.
pub fn settings(cx: &App) -> FindInFilesSettings {
    cx.try_global::<SettingsStore>()
        .map(|s| s.settings.clone())
        .unwrap_or_default()
}

/// Changes the settings and writes them in the background.
fn update_settings(cx: &mut App, f: impl FnOnce(&mut FindInFilesSettings)) {
    let store = cx.default_global::<SettingsStore>();
    let before = store.settings.clone();
    f(&mut store.settings);
    if store.settings == before {
        return;
    }
    let Some(file) = store.file.clone() else {
        return;
    };
    let Ok(text) = toml::to_string(&store.settings) else {
        return;
    };
    cx.background_spawn(async move {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, text) {
            eprintln!("cannot save {}: {e}", file.display());
        }
    })
    .detach();
}

/// What the popup tells the workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindInFilesEvent {
    /// Open a file with the caret at a point.
    OpenAt { path: PathBuf, point: Point },
    /// The popup was closed with Escape: the focus goes back to where it
    /// was.
    Dismissed,
}

impl EventEmitter<FindInFilesEvent> for FindInFiles {}

/// What the popup is opened with.
#[derive(Debug, Clone, Default)]
pub struct OpenContext {
    pub root: Option<PathBuf>,
    /// The project's hide and exclude rules as the project tool window has
    /// them (`None`: the rules of the root).
    pub filter: Option<project::Filter>,
    /// Text selected in the active editor.
    pub selection: Option<String>,
    /// The search of the active editor's open find bar.
    pub find_bar_text: Option<String>,
    /// Open documents with unsaved changes, searched instead of their files.
    pub overrides: HashMap<PathBuf, Rope>,
}

/// Where a search stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// Nothing to search for.
    Empty,
    /// Waiting for more typing, or searching.
    Searching,
    Done,
    /// The query or the mask cannot be used.
    Invalid(String),
    /// No project is open.
    NoProject,
}

/// The Find in Files popup.
pub struct FindInFiles {
    focus: FocusHandle,
    query: Entity<Editor>,
    mask: Entity<Editor>,
    open: bool,
    options: SearchOptions,
    mask_on: bool,
    pinned: bool,
    root: Option<PathBuf>,
    /// The project's hide and exclude rules (`None`: read from the root
    /// when a search starts).
    filter: Option<project::Filter>,
    overrides: HashMap<PathBuf, Rope>,
    results: Results,
    /// The rows belong to the previous search; they go when the first row
    /// of the new one arrives.
    stale: bool,
    selected: Option<usize>,
    status: Status,
    /// The search stopped at the limit.
    truncated: bool,
    /// Files skipped as too large by the last search.
    large_files: usize,
    /// Project configuration files whose matches the last search held back.
    held_back: usize,
    generation: u64,
    cancel: Option<CancelToken>,
    search_task: Option<Task<()>>,
    debounce: Option<Task<()>>,
    list: UniformListScrollHandle,
    /// Moved by dragging the header: offset from the default place.
    offset: gpui::Point<Pixels>,
    /// Dragging: the mouse position and the offset when it started.
    drag: Option<(gpui::Point<Pixels>, gpui::Point<Pixels>)>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for FindInFiles {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FindInFiles {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut e = Editor::new(Buffer::new(""), window, cx);
                e.set_single_line(true);
                e.set_ui_font(true);
                e.set_transparent(true);
                e.set_placeholder(Some(placeholder.into()));
                e.set_config(editor::EditorConfig {
                    brackets: Vec::new(),
                    ..editor::EditorConfig::default()
                });
                e
            })
        };
        let query = field("Search", window, cx);
        let mask = field("", window, cx);
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) {
                    this.query.update(cx, |f, cx| f.set_error(false, cx));
                    this.schedule(cx);
                }
            }),
            cx.subscribe(&mask, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) && this.mask_on {
                    this.schedule(cx);
                }
            }),
            // Clicking elsewhere in the window closes the popup, unless it
            // is pinned; switching to another application does not.
            cx.on_focus_out(&focus, window, |this, _, window, cx| {
                if this.open && !this.pinned && window.is_window_active() {
                    this.close(false, window, cx);
                }
            }),
        ];
        let settings = settings(cx);
        let shown_mask = settings.shown_mask();
        mask.update(cx, |f, cx| f.set_text_selected(&shown_mask, cx));
        Self {
            focus,
            query,
            mask,
            open: false,
            options: settings.options(),
            mask_on: settings.file_mask.is_some(),
            pinned: settings.pinned,
            root: None,
            filter: None,
            overrides: HashMap::new(),
            results: Results::new(),
            stale: false,
            selected: None,
            status: Status::Empty,
            truncated: false,
            large_files: 0,
            held_back: 0,
            generation: 0,
            cancel: None,
            search_task: None,
            debounce: None,
            list: UniformListScrollHandle::new(),
            offset: point(px(0.0), px(0.0)),
            drag: None,
            _subscriptions: subscriptions,
        }
    }

    // ---- state ------------------------------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn query_field(&self) -> &Entity<Editor> {
        &self.query
    }

    pub fn mask_field(&self) -> &Entity<Editor> {
        &self.mask
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

    /// The counter of the header: `3 matches in 2 files`, with `+` when the
    /// search stopped at its limit; empty without matches.
    pub fn info(&self) -> String {
        let (m, f) = (self.results.matches(), self.results.files());
        if m == 0 || self.stale {
            return String::new();
        }
        let plus = if self.truncated && self.status == Status::Done {
            "+"
        } else {
            ""
        };
        let matches = if m == 1 { "match" } else { "matches" };
        let files = if f == 1 { "file" } else { "files" };
        format!("{m}{plus} {matches} in {f}{plus} {files}")
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

    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let rel = |p: &Path| -> String {
            self.root
                .as_ref()
                .and_then(|r| p.strip_prefix(r).ok())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let rows: Vec<_> = if self.stale {
            Vec::new()
        } else {
            self.results
                .rows()
                .iter()
                .map(|r| {
                    json!({
                        "path": rel(&r.path),
                        "line": r.line.line + 1,
                        "text": r.line.text,
                        "ranges": r.line.ranges.iter().map(|m| [m.start, m.end]).collect::<Vec<_>>(),
                    })
                })
                .collect()
        };
        json!({
            "open": self.open,
            "text": self.query.read(cx).text(),
            "case_sensitive": self.options.case_sensitive,
            "whole_words": self.options.whole_words,
            "regex": self.options.regex,
            "mask": { "on": self.mask_on, "text": self.mask.read(cx).text() },
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
        })
    }

    // ---- opening and closing ----------------------------------------------

    /// Opens the popup (or refreshes it when open): the selected text, else
    /// the search of an open find bar, else the last search fills the field,
    /// selected; the project is searched.
    pub fn open(&mut self, context: OpenContext, window: &mut Window, cx: &mut Context<Self>) {
        self.root = context.root;
        self.filter = context.filter;
        self.overrides = context.overrides;
        let mut text = self.query.read(cx).text();
        if let Some(selected) = context
            .selection
            .filter(|s| !s.is_empty() && s.len() < 10_000)
        {
            text = if self.options.regex {
                search::escape(&selected)
            } else {
                selected
            };
        } else if let Some(bar) = context.find_bar_text {
            text = bar;
        } else if text.is_empty() {
            text = editor::find::last_search(cx)
                .history
                .first()
                .cloned()
                .unwrap_or_default();
        }
        self.open = true;
        self.query
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        window.focus(&self.query.focus_handle(cx));
        self.search_now(cx);
        cx.notify();
    }

    /// Opens the popup and searches for `text` with the given options.
    pub fn search_for(
        &mut self,
        args: &Search,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = SearchOptions {
            case_sensitive: args.case_sensitive.unwrap_or(self.options.case_sensitive),
            whole_words: args.whole_words.unwrap_or(self.options.whole_words),
            regex: args.regex.unwrap_or(self.options.regex),
        };
        self.set_options(options, cx);
        if let Some(mask) = &args.mask {
            self.mask_on = !mask.is_empty();
            if !mask.is_empty() {
                self.mask.update(cx, |f, cx| f.set_text_selected(mask, cx));
            }
        }
        self.open(context, window, cx);
        self.query
            .update(cx, |f, cx| f.set_text_selected(&args.text, cx));
        self.search_now(cx);
    }

    /// Closes the popup; with `restore_focus` (Escape) the focus goes back
    /// to where it was. The search is stopped, the settings are saved.
    pub fn close(&mut self, restore_focus: bool, _: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.drag = None;
        self.cancel_search();
        self.save(cx);
        if restore_focus {
            cx.emit(FindInFilesEvent::Dismissed);
        }
        cx.notify();
    }

    /// Remembers the search and the mask (the reference saves its settings
    /// when the popup closes or a result is opened).
    fn save(&mut self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).text();
        editor::find::remember_search(cx, &text);
        let mask = self.mask.read(cx).text();
        let (mask_on, options, pinned) = (self.mask_on, self.options, self.pinned);
        update_settings(cx, |s| {
            s.file_mask = mask_on.then(|| mask.clone());
            if mask_on {
                s.remember_mask(&mask);
            }
            s.case_sensitive = options.case_sensitive;
            s.whole_words = options.whole_words;
            s.regex = options.regex;
            s.pinned = pinned;
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

    /// What is searched for, or why nothing is.
    fn build_search(&self, cx: &App) -> Result<FileSearch, Status> {
        let text = self.query.read(cx).text();
        let matcher = match Query::new(&text, self.options).compile() {
            Ok(m) => m,
            Err(QueryError::Empty) => return Err(Status::Empty),
            Err(QueryError::Vague) => {
                return Err(Status::Invalid(
                    "Regular expression matches empty string".into(),
                ));
            }
            Err(QueryError::InvalidRegex(message)) => {
                return Err(Status::Invalid(format!(
                    "Bad pattern \"{text}\": {message}"
                )));
            }
        };
        if self.options.regex && matches_empty(&text) {
            return Err(Status::Invalid(
                "Regular expression matches empty string".into(),
            ));
        }
        let mask = if self.mask_on {
            let mask = self.mask.read(cx).text();
            if mask.trim().is_empty() {
                return Err(Status::Invalid("Empty file mask".into()));
            }
            if mask.contains(';') {
                return Err(Status::Invalid(
                    "File masks should be comma-separated".into(),
                ));
            }
            Some(
                FileMask::parse(&mask)
                    .map_err(|_| Status::Invalid(format!("Bad file mask \"{mask}\"")))?,
            )
        } else {
            None
        };
        let root = self.root.clone().ok_or(Status::NoProject)?;
        Ok(FileSearch {
            matcher,
            mask,
            scope: Scope::Directory {
                path: root.clone(),
                recursive: true,
            },
            root,
            filter: self.filter.clone(),
            limit: PAGE_SIZE,
            overrides: self.overrides.clone(),
            project_files: false,
        })
    }

    /// Starts a search now, cancelling the running one.
    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        self.generation += 1;
        let search = match self.build_search(cx) {
            Ok(search) => search,
            Err(status) => {
                let query_error = matches!(&status, Status::Invalid(m) if !m.contains("mask"));
                self.query.update(cx, |f, cx| f.set_error(query_error, cx));
                self.status = status;
                self.results = Results::new();
                self.stale = false;
                self.selected = None;
                self.truncated = false;
                cx.notify();
                return;
            }
        };
        self.query.update(cx, |f, cx| f.set_error(false, cx));
        self.status = Status::Searching;
        self.stale = true;
        self.truncated = false;
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
                this.truncated = summary.truncated;
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
            self.results = Results::new();
            self.stale = false;
            self.selected = None;
        }
        let selected = self
            .selected
            .and_then(|i| self.results.row(i))
            .map(|r| (r.path.clone(), r.line.line));
        for file in files {
            self.results.add(file);
        }
        self.selected = match selected {
            Some((path, line)) => self.results.position(&path, line),
            None if !self.results.is_empty() => {
                self.list.scroll_to_item(0, ScrollStrategy::Top);
                Some(0)
            }
            None => None,
        };
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
    fn up_down(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.query.read(cx).buffer().len_lines() > 1 {
            cx.propagate();
            return;
        }
        self.move_selection(delta, cx);
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
        let event = FindInFilesEvent::OpenAt {
            path: (*row.path).clone(),
            point: row.line.first.start,
        };
        self.save(cx);
        if !self.pinned {
            self.close(false, window, cx);
        }
        cx.emit(event);
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

    // ---- actions ----------------------------------------------------------

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        self.close(true, window, cx);
    }

    fn next_result_action(&mut self, _: &NextResult, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn previous_result_action(
        &mut self,
        _: &PreviousResult,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    fn down_action(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.up_down(1, cx);
    }

    fn up_action(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.up_down(-1, cx);
    }

    fn page_down_action(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(PAGE_ROWS as isize, cx);
    }

    fn page_up_action(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-(PAGE_ROWS as isize), cx);
    }

    fn open_result_action(&mut self, _: &OpenResult, window: &mut Window, cx: &mut Context<Self>) {
        self.open_result(window, cx);
    }

    fn toggle_mask_action(&mut self, _: &ToggleMask, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mask(window, cx);
    }

    fn toggle_pin_action(&mut self, _: &TogglePin, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_pin(cx);
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
        window.focus(&self.query.focus_handle(cx));
        self.query.update(cx, |f, cx| f.insert("\n", cx));
    }

    // ---- drawing ----------------------------------------------------------

    /// Where the popup goes in a window of this size.
    fn bounds(&self, viewport: gpui::Size<Pixels>) -> gpui::Bounds<Pixels> {
        let width = px(WIDTH)
            .min(viewport.width - px(2.0 * MARGIN))
            .max(px(320.0));
        let height = px(HEIGHT)
            .min(viewport.height - px(TOP + MARGIN))
            .max(px(240.0));
        let x = ((viewport.width - width) / 2.0 + self.offset.x)
            .clamp(px(0.0), (viewport.width - width).max(px(0.0)));
        let y = (px(TOP) + self.offset.y).clamp(px(0.0), (viewport.height - px(40.0)).max(px(0.0)));
        gpui::Bounds::new(point(x, y), gpui::size(width, height))
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focus = self.query.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &focus, window).map(SharedString::from)
        };
        let mask_focused = self.mask.focus_handle(cx).is_focused(window);
        let mask_field = input_frame(mask_focused, false, cx)
            .id("find-in-files-mask")
            .debug_selector(|| "find-in-files-mask".into())
            .key_context("FindInFilesMask")
            .w(px(MASK_WIDTH))
            .h(px(SMALL_BUTTON + 4.0))
            .px(px(6.0))
            .when(!self.mask_on, |d| d.opacity(0.5))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(editor::field_line_height(window, cx))
                    .overflow_hidden()
                    .child(self.mask.clone()),
            );
        div()
            .id("find-in-files-header")
            .debug_selector(|| "find-in-files-header".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(HEADER_HEIGHT))
            .px(px(PAD))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.drag = Some((e.position, this.offset));
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Find in Files"),
            )
            .child(
                div()
                    .id("find-in-files-info")
                    .debug_selector(|| "find-in-files-info".into())
                    .flex_none()
                    .pl(px(4.0))
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(self.info()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("find-in-files-mask-toggle")
                    .debug_selector(|| "find-in-files-mask-toggle".into())
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .child(checkbox("find-in-files-mask-box", self.mask_on, cx))
                    .child("File mask:")
                    .tooltip(Tooltip::text("File mask", shortcut(&ToggleMask)))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.toggle_mask(window, cx)
                        }),
                    ),
            )
            .child(mask_field)
            .child(
                toggle_button("find-in-files-pin", IconName::PIN, self.pinned, cx)
                    .tooltip(Tooltip::text("Pin Window", shortcut(&TogglePin)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_pin(cx))),
            )
    }

    fn render_search_field(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focus = self.query.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &focus, window).map(SharedString::from)
        };
        let line = editor::field_line_height(window, cx);
        let rows = self
            .query
            .read(cx)
            .buffer()
            .len_lines()
            .clamp(1, editor::find::MAX_FIELD_ROWS);
        let pad = ((px(FIELD_HEIGHT) - line) / 2.0).max(px(0.0));
        let height = px(FIELD_HEIGHT) + line * (rows - 1) as f32;
        let error = self.query.read(cx).is_error();
        let new_line = {
            let target = self.query.clone();
            small_icon_button("find-in-files-new-line", IconName::NEW_LINE, true, cx)
                .tooltip(Tooltip::text("New Line", shortcut(&editor::find::NewLine)))
                .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                    window.focus(&target.focus_handle(cx));
                    target.update(cx, |f, cx| f.insert("\n", cx));
                }))
        };
        let options = self.options;
        div()
            .id("find-in-files-query")
            .debug_selector(|| "find-in-files-query".into())
            .key_context("FindInFilesQuery")
            .flex()
            .flex_row()
            .flex_none()
            .items_start()
            .h(height)
            .px(px(PAD - 4.0))
            .gap(px(4.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            .when(error, |d| d.bg(theme.color(UiRole::DangerBg)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(FIELD_HEIGHT))
                    .px(px(4.0))
                    .child(Icon::new(IconName::SEARCH_HISTORY)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .mt(pad)
                    .h(line * rows as f32)
                    .overflow_hidden()
                    .child(self.query.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(px(FIELD_HEIGHT))
                    .gap(px(2.0))
                    .child(new_line)
                    .child(
                        toggle_button(
                            "find-in-files-match-case",
                            IconName::MATCH_CASE,
                            options.case_sensitive,
                            cx,
                        )
                        .tooltip(Tooltip::text(
                            "Match Case",
                            shortcut(&editor::find::ToggleCase),
                        ))
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.toggle_option(|o| o.case_sensitive = !o.case_sensitive, cx)
                            },
                        )),
                    )
                    .child(
                        toggle_button(
                            "find-in-files-words",
                            IconName::WORDS,
                            options.whole_words,
                            cx,
                        )
                        .tooltip(Tooltip::text("Words", shortcut(&editor::find::ToggleWords)))
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.toggle_option(|o| o.whole_words = !o.whole_words, cx)
                            },
                        )),
                    )
                    .child(
                        toggle_button("find-in-files-regex", IconName::REGEX, options.regex, cx)
                            .tooltip(Tooltip::text("Regex", shortcut(&editor::find::ToggleRegex)))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toggle_option(|o| o.regex = !o.regex, cx)
                            })),
                    ),
            )
    }

    fn render_scopes(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .h(px(SCOPE_HEIGHT))
            .px(px(PAD - 4.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            .child(
                div()
                    .id("find-in-files-scope-project")
                    .debug_selector(|| "find-in-files-scope-project".into())
                    .flex()
                    .items_center()
                    .h(px(SMALL_BUTTON + 2.0))
                    .px(px(8.0))
                    .rounded(theme.metric(Metric::CornerRadius))
                    .bg(theme.color(UiRole::SearchOptionSelected))
                    .child("In Project"),
            )
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let Some(row) = self.results.row(index) else {
            return div().into_any_element();
        };
        let selected = self.selected == Some(index);
        let (text, highlights) = row_text(row, theme.color(UiRole::SearchMatch));
        let file_name = row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let location = format!("{file_name} {}", row.line.line + 1);
        div()
            .id(("find-in-files-row", index))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(row_height)
            .px(px(PAD))
            .gap(px(12.0))
            .when(selected, |d| d.bg(theme.color(UiRole::Selection)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    // The editor font, with ligatures as the editor has them.
                    .font(gpui::Font {
                        family: theme.mono_family(),
                        features: if theme.raw().fonts.editor_ligatures {
                            gpui::FontFeatures::default()
                        } else {
                            gpui::FontFeatures::disable_ligatures()
                        },
                        fallbacks: None,
                        weight: FontWeight::NORMAL,
                        style: gpui::FontStyle::Normal,
                    })
                    .child(StyledText::new(text).with_highlights(highlights)),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(location),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                this.select(index, cx);
                if e.click_count() >= 2 {
                    this.open_result(window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_results(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        if let Some(text) = self.empty_text() {
            return div()
                .id("find-in-files-empty")
                .debug_selector(|| "find-in-files-empty".into())
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(text)
                .into_any_element();
        }
        let count = self.results.len();
        let mut rows = uniform_list(
            "find-in-files-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "find-in-files-rows".into())
        .track_scroll(self.list.clone())
        .flex_1()
        .py(px(4.0))
        .overflow_x_hidden();
        rows.style().restrict_scroll_to_axis = Some(true);
        rows.into_any_element()
    }
}

/// A row's text without its indentation (tabs as spaces), and the matches
/// as byte ranges of it with their background.
fn row_text(
    row: &project_search::Row,
    background: gpui::Hsla,
) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let chars: Vec<char> = row.line.text.chars().collect();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let text: String = chars[indent..]
        .iter()
        .map(|&c| if c == '\t' { ' ' } else { c })
        .collect();
    // Byte offset of each char of `text`, and the end.
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let start = row.line.text_column + indent;
    let style = HighlightStyle {
        background_color: Some(background),
        ..HighlightStyle::default()
    };
    let highlights = row
        .line
        .ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.saturating_sub(start).min(offsets.len() - 1);
            let b = r.end.saturating_sub(start).min(offsets.len() - 1);
            (b > a).then(|| (offsets[a]..offsets[b], style))
        })
        .collect();
    (text, highlights)
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
    ToggleMask => toggle_mask_action,
    TogglePin => toggle_pin_action,
    editor::find::ToggleCase => toggle_case_action,
    editor::find::ToggleWords => toggle_words_action,
    editor::find::ToggleRegex => toggle_regex_action,
    editor::find::NewLine => new_line_action,
}

impl Render for FindInFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let bounds = self.bounds(window.viewport_size());
        let dragging = self.drag.is_some().then(|| {
            let this = cx.entity();
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let moved = this.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, _, _, cx| {
                        moved.update(cx, |this, cx| {
                            if let Some((start, offset)) = this.drag {
                                this.offset = offset + (e.position - start);
                                cx.notify();
                            }
                        })
                    });
                    let released = this.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                        released.update(cx, |this, cx| {
                            this.drag = None;
                            cx.notify();
                        })
                    });
                },
            )
            .absolute()
            .size_0()
        });
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            // Clicks inside the popup leave the focus in its fields.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(self.render_header(window, cx))
            .child(self.render_search_field(window, cx))
            .child(self.render_scopes(cx))
            .child(self.render_results(cx));
        let theme = cx.theme();
        let el = div()
            .id("find-in-files")
            .debug_selector(|| "find-in-files".into())
            .key_context("FindInFiles")
            .track_focus(&self.focus)
            .occlude()
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .border_1()
            .border_color(theme.color(UiRole::Border))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow_lg()
            .child(content)
            .children(dragging);
        Self::bind_actions(el, cx).into_any_element()
    }
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
    fn masks_are_remembered_newest_first() {
        let mut s = FindInFilesSettings::with_masks(&["*.py", "*.xml"]);
        assert_eq!(s.shown_mask(), "*.py");
        s.remember_mask("*.txt");
        s.remember_mask("*.py");
        assert_eq!(s.recent_masks, ["*.py", "*.txt", "*.xml"]);
        s.file_mask = Some("*.txt".into());
        assert_eq!(s.shown_mask(), "*.txt");
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<FindInFilesSettings>(&text).unwrap(), s);
        // A file from an older version: what is missing takes its default.
        let old: FindInFilesSettings = toml::from_str("pinned = true").unwrap();
        assert!(old.pinned && old.recent_masks.is_empty());
    }

    #[test]
    fn row_text_drops_the_indentation_and_maps_matches_to_bytes() {
        let row = project_search::Row {
            path: std::sync::Arc::new(PathBuf::from("a.py")),
            line: project_search::LineMatch {
                line: 0,
                text: "\t  ä = \"ä\"".into(),
                text_column: 0,
                ranges: vec![3..4, 8..9],
                first: Point::new(0, 3)..Point::new(0, 4),
            },
        };
        let (text, highlights) = row_text(&row, gpui::Hsla::default());
        assert_eq!(text, "ä = \"ä\"");
        let ranges: Vec<_> = highlights.into_iter().map(|(r, _)| r).collect();
        assert_eq!(ranges, vec![0..2, 6..8]);
    }
}
