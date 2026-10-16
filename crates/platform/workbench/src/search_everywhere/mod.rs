//! Search Everywhere and Go to File: one popup with tabs (All, Files,
//! Actions, Text), as the reference IDE's current ("split") Search
//! Everywhere (tests/scenarios/editor/search-everywhere.md).
//!
//! Double Shift opens it on All, Go to File on Files, Go to Action on
//! Actions, Find Text on Text; pressing the shortcut of the tab shown again
//! switches its scope (All: "Include non-project items", Files and Text:
//! Project Files / All Places, Actions: "Include disabled actions"), another
//! one switches the tab. The field starts with the text selected in the
//! editor, else the tab's last search, selected.
//!
//! - Files: the project's file name index ([`crate::project_files`]) finds
//!   files and folders as the reference does and ranks them by where the
//!   name matches (`file_index::rank`); rows show a location from the
//!   product (a Python package path) and package folders; the footer the
//!   path from its source root. `name:line:column` opens at that place. A
//!   filter leaves out file types; the scope (persisted) adds non-project
//!   items.
//! - Actions: the registered commands by title ([`actions`]).
//! - Text: lines of the project's files (`project-search`), with Match
//!   Case, Words and Regex and the file mask shared with Find in Files.
//! - All: recently opened files that match on top, then everything by
//!   weight (text last); with an empty field the recent files.
//!
//! A tab shows a page of each kind (15, on Actions and Text 30, as the
//! reference's legacy popup does) and a "… more" row that shows the next
//! page. Typing searches again [`DEBOUNCE`] after the last key
//! (at once for an empty field); the rows shown stay until the new ones
//! come. A search that finds nothing widens the tab's scope by itself,
//! until the text no longer contains the text that found nothing.

pub mod actions;
mod preview;
mod providers;
mod settings;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use editor::Editor;
use file_index::{FileIndex, parse_position};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, Subscription, Task, UniformListScrollHandle, Window, prelude::*, px,
};
use project_search::{FileMask, FileSearch, Scope};
use search::{CancelToken, Query, SearchOptions};
use serde_json::json;
use text::Buffer;
use ui::theme::ThemeExt as _;

use crate::file_types::FileTypes;
use crate::popup::PopupFrame;
use actions::ActionItem;
use providers::{DIRECTORY_TYPE, Found};
pub use providers::{FileRow, Provider, Row, TextRow};
pub(crate) use settings::update_settings;
pub use settings::{SearchEverywhereSettings, load_settings, settings, settings_file};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Search Everywhere (All tab).
    Open => "search-everywhere.open", "Search Everywhere",
        "Opens Search Everywhere on the All tab: recent files, files, actions and text.";
    /// Go to File.
    GoToFile => "search-everywhere.go-to-file", "Go to File…",
        "Opens Search Everywhere on the Files tab to open a file by its name (`name:line:column` opens at a line).";
    /// Go to Action.
    GoToAction => "search-everywhere.go-to-action", "Find Action…",
        "Opens Search Everywhere on the Actions tab to run a command by its name.";
    /// Find text.
    GoToText => "search-everywhere.go-to-text", "Text…",
        "Opens Search Everywhere on the Text tab to find a text in the project's files.";
    /// Search for a text in a tab.
    Search {
        text: String,
        tab: Option<String>,
        non_project: Option<bool>,
    } => "search-everywhere.search", "Search Everywhere for Text",
        "Opens Search Everywhere on a tab (`all`, `files`, `actions`, `text`) with a text; `non_project` widens the tab's scope (non-project items, All Places, disabled actions).",
        r#"{"text": "app.py:12", "tab": "files", "non_project": false}"#;
    /// Describe the popup.
    State => "search-everywhere.state", "Search Everywhere State",
        "Returns the state of the Search Everywhere popup as JSON (tab, text, scope, rows, selected row).";
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "SearchEverywhere";
    /// Close the popup.
    Close => "search-everywhere.close", "Close", "Closes Search Everywhere.";
    /// The next tab.
    NextTab => "search-everywhere.next-tab", "Next Tab", "Shows the next tab (after the last, the first).";
    /// The previous tab.
    PreviousTab => "search-everywhere.previous-tab", "Previous Tab", "Shows the previous tab (before the first, the last).";
    /// The next row.
    Down => "search-everywhere.down", "Next Result", "Selects the next result (after the last, the first).";
    /// The previous row.
    Up => "search-everywhere.up", "Previous Result", "Selects the previous result (before the first, the last once all are there).";
    /// A page down.
    NextGroup => "search-everywhere.next-group", "Page Down", "Selects the result a page further down.";
    /// The first row.
    PreviousGroup => "search-everywhere.previous-group", "First Result", "Selects the first result.";
    /// Open or run the selected row.
    Select => "search-everywhere.select", "Open Result",
        "Opens the selected file (at the line and column typed after its name) or line, or runs the selected action.";
    /// The scope list (its mnemonic).
    ChooseScope => "search-everywhere.choose-scope", "Choose Scope",
        "Opens the scope list of the Files or Text tab.";
    /// Widen the scope.
    ToggleEverywhere => "search-everywhere.toggle-everywhere", "Include Non-Project Items",
        "Switches the tab's scope: non-project items on All, All Places on Files and Text, disabled actions on Actions.";
    /// The previous search of the tab.
    HistoryPrevious => "search-everywhere.history-previous", "Previous Search",
        "Puts the tab's previous search in the field.";
    /// The next search of the tab.
    HistoryNext => "search-everywhere.history-next", "Next Search",
        "Puts the tab's next search in the field.";
    /// Match Case on the Text tab.
    ToggleCase => "search-everywhere.toggle-case", "Match Case",
        "Switches Match Case of the Text tab (shared with Find in Files).";
    /// Words on the Text tab.
    ToggleWords => "search-everywhere.toggle-words", "Words",
        "Switches Words of the Text tab (shared with Find in Files).";
    /// Regex on the Text tab.
    ToggleRegex => "search-everywhere.toggle-regex", "Regex",
        "Switches Regex of the Text tab (shared with Find in Files).";
    /// The preview under the list.
    TogglePreview => "search-everywhere.toggle-preview", "Preview",
        "Shows or hides the preview of the selected item's file under the list (remembered).";
}

/// Rows of each kind a page shows: on All and Files (the reference's Files
/// tab has more than one kind of result), and on the other tabs.
pub const PAGE: usize = 15;
pub const SINGLE_PAGE: usize = 30;
/// Wait after a key before searching (the reference's).
pub const DEBOUNCE: Duration = Duration::from_millis(100);
/// Searches kept per tab.
const HISTORY_LIMIT: usize = 50;
/// Default size (the reference's minimum width and list height); the
/// popup is centred and a quarter down the window.
const WIDTH: f32 = 700.0;
const HEIGHT: f32 = 600.0;
const MIN_WIDTH: f32 = 700.0;
/// The header with the tabs, and the field.
const HEADER_HEIGHT: f32 = 40.0;
const FIELD_HEIGHT: f32 = 40.0;
/// The list is at least this high when shown.
const MIN_LIST_HEIGHT: f32 = 100.0;
/// Rows a page down leaves of the previous page.
const PAGE_OVERLAP: usize = 4;

/// The tabs, in the reference's order (it shows Classes and Symbols when a
/// language provides them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    All,
    Files,
    Actions,
    Text,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::All, Tab::Files, Tab::Actions, Tab::Text];

    pub fn title(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Files => "Files",
            Tab::Actions => "Actions",
            Tab::Text => "Text",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tab::All => "all",
            Tab::Files => "files",
            Tab::Actions => "actions",
            Tab::Text => "text",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The command that opens the popup on this tab.
    fn command(self) -> &'static str {
        match self {
            Tab::All => "search-everywhere.open",
            Tab::Files => "search-everywhere.go-to-file",
            Tab::Actions => "search-everywhere.go-to-action",
            Tab::Text => "search-everywhere.go-to-text",
        }
    }

    fn providers(self) -> &'static [Provider] {
        match self {
            Tab::All => &[
                Provider::Recent,
                Provider::Files,
                Provider::Actions,
                Provider::Text,
            ],
            Tab::Files => &[Provider::Files],
            Tab::Actions => &[Provider::Actions],
            Tab::Text => &[Provider::Text],
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What the popup opens with (the workbench's state).
#[derive(Default, Clone)]
pub struct OpenContext {
    pub root: Option<PathBuf>,
    pub index: Option<Arc<FileIndex>>,
    /// The project's rules (for the Text tab).
    pub filter: Option<project::Filter>,
    /// The editor's selection (one line), for the field.
    pub selection: Option<String>,
    pub open_files: Vec<PathBuf>,
    /// The file of the active editor.
    pub current_file: Option<PathBuf>,
    /// Files opened in editors, most recent first.
    pub recent_files: Vec<PathBuf>,
    /// The commands, as available where the focus was.
    pub actions: Vec<ActionItem>,
    /// Open documents with unsaved changes (searched instead of the file).
    pub overrides: HashMap<PathBuf, text::Rope>,
    /// The project's source roots (locations, the footer's path).
    pub source_roots: Arc<Vec<PathBuf>>,
    /// What the product's language adds (locations, package folders).
    pub navigation: crate::app::NavigationHooks,
    /// The home folder (`~/` paths).
    pub home: Option<PathBuf>,
}

/// What the popup asks of the workbench.
#[derive(Debug, Clone)]
pub enum SearchEverywhereEvent {
    /// Open a file, at a zero-based line and column when given.
    OpenFile {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
    /// Show a folder in the project tool window.
    RevealFolder(PathBuf),
    /// Run a command where the focus was.
    Run(&'static str),
    /// Closed with Escape: the focus goes back.
    Dismissed,
    /// Non-project items are wanted: the index lists excluded folders too.
    WantNonProject,
    /// Search the text with Find in Files (from the empty list's link).
    FindInFiles(String),
}

impl EventEmitter<SearchEverywhereEvent> for SearchEverywhere {}

/// A list over the popup: the scope list, a filter, the search history or
/// the file masks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Dropdown {
    Scope,
    FileTypes,
    Providers,
    History,
    Masks,
}

/// A row as it is kept across a reopen (to select it again).
#[derive(Debug, Clone, PartialEq)]
enum RowKey {
    Path(PathBuf),
    Line(PathBuf, usize),
}

pub struct SearchEverywhere {
    focus: FocusHandle,
    field: Entity<Editor>,
    file_types: Rc<FileTypes>,
    open: bool,
    tab: Tab,
    context: OpenContext,
    /// The scope of each tab is widened (non-project items, All Places,
    /// disabled actions).
    everywhere: [bool; 4],
    /// The person set the scope in this popup: it is not changed by itself.
    chosen: [bool; 4],
    /// Widened by the popup because the text found nothing.
    auto_everywhere: Option<String>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// The person moved the selection since the text changed: it stays on
    /// its row (else the first row is selected as rows come).
    selection_moved: bool,
    /// Rows of each kind asked for (grows as the list scrolls).
    limits: HashMap<Provider, usize>,
    /// Kinds that have more rows than shown.
    more: HashSet<Provider>,
    /// Rows shown before loading more stay where they are.
    frozen: usize,
    searching: bool,
    generation: u64,
    search_task: Option<Task<()>>,
    cancel: Option<CancelToken>,
    debounce: Option<Task<()>>,
    /// The Text tab's options and file mask (`None`: all file types),
    /// shared with Find in Files.
    options: SearchOptions,
    mask: Option<String>,
    /// Searches per tab, oldest first, and the position while going
    /// through them.
    history: Vec<(String, Tab)>,
    history_at: Option<usize>,
    /// The tab, text and selected row when the popup closed.
    last_selection: Option<(Tab, String, RowKey)>,
    dropdown: Option<Dropdown>,
    list: UniformListScrollHandle,
    frame: PopupFrame,
    /// Checks in the background which recent files still exist.
    recent_check: Option<Task<()>>,
    /// The selected row's file under the list.
    preview: preview::Preview,
    /// Measurements: when the last search started and how long it took.
    started: Option<std::time::Instant>,
    finished: Option<Duration>,
    /// Version control statuses of the file names (#169).
    file_colors: vcs_ui::FileColors,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SearchEverywhere {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SearchEverywhere {
    pub fn new(file_types: Rc<FileTypes>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.set_single_line(true);
            e.set_ui_font(true);
            e.set_transparent(true);
            e.set_config(editor::EditorConfig {
                brackets: Vec::new(),
                ..editor::EditorConfig::default()
            });
            e
        });
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe(&field, |this, _, event, cx| {
                if matches!(event, editor::EditorEvent::Edited(_)) {
                    this.history_at = None;
                    this.dropdown = None;
                    this.text_changed(cx);
                }
            }),
            // Clicking elsewhere in the window closes the popup; switching
            // to another application does not.
            cx.on_focus_out(&focus, window, |this, _, window, cx| {
                if this.open && window.is_window_active() {
                    this.close(false, window, cx);
                }
            }),
        ];
        let mut frame = PopupFrame::new(
            gpui::size(px(WIDTH), px(HEIGHT)),
            gpui::size(
                px(MIN_WIDTH),
                px(HEADER_HEIGHT + FIELD_HEIGHT + MIN_LIST_HEIGHT),
            ),
            px(0.0),
        );
        if let Some(place) = settings(cx).popup {
            frame.restore(place);
        }
        let preview = preview::Preview::new(file_types.clone(), window, cx);
        Self {
            focus,
            field,
            file_types,
            open: false,
            tab: Tab::All,
            context: OpenContext::default(),
            everywhere: [false; 4],
            chosen: [false; 4],
            auto_everywhere: None,
            rows: Vec::new(),
            selected: None,
            selection_moved: false,
            limits: HashMap::new(),
            more: HashSet::new(),
            frozen: 0,
            searching: false,
            generation: 0,
            search_task: None,
            cancel: None,
            debounce: None,
            options: SearchOptions::default(),
            mask: None,
            history: Vec::new(),
            history_at: None,
            last_selection: None,
            dropdown: None,
            list: UniformListScrollHandle::new(),
            frame,
            recent_check: None,
            preview,
            started: None,
            finished: None,
            file_colors: vcs_ui::FileColors::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The statuses the file names are coloured with.
    pub fn set_file_colors(&mut self, colors: vcs_ui::FileColors) {
        self.file_colors = colors;
    }

    // ---- state ------------------------------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn field(&self) -> &Entity<Editor> {
        &self.field
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The last search's time to its results (measurements).
    pub fn timing(&self) -> Option<Duration> {
        self.finished
    }

    pub fn is_searching(&self) -> bool {
        self.searching || self.debounce.is_some()
    }

    fn everywhere(&self) -> bool {
        self.everywhere[self.tab.index()]
    }

    /// The scope's name on the Files and Text tabs.
    fn scope_name(&self) -> &'static str {
        if self.everywhere() {
            "All Places"
        } else {
            "Project Files"
        }
    }

    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let rows: Vec<_> = self.rows.iter().map(|r| self.row_json(r)).collect();
        json!({
            "open": self.open,
            "tab": self.tab.name(),
            "tabs": Tab::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
            "text": self.field.read(cx).text(),
            "everywhere": self.everywhere(),
            "scope": matches!(self.tab, Tab::Files | Tab::Text).then(|| self.scope_name()),
            "options": {
                "case_sensitive": self.options.case_sensitive,
                "whole_words": self.options.whole_words,
                "regex": self.options.regex,
                "mask": self.mask,
            },
            "hidden_file_types": settings(cx).hidden_file_types,
            "hidden_providers": settings(cx).hidden_providers,
            "searching": self.is_searching(),
            "collapsed": self.collapsed(cx),
            "rows": rows,
            "selected": self.selected,
            "dropdown": self.dropdown.as_ref().map(|d| format!("{d:?}").to_lowercase()),
            "footer": self.footer().0,
            "preview": {
                "on": settings(cx).preview,
                "shown": self.preview_shown(cx),
                "view": self.preview.view.read(cx).state_json(|p| self.rel(p), cx),
                "split": self.preview.split(),
            },
            "dragging": {
                "popup": self.frame.is_dragging(),
                "split": self.preview.is_dragging(),
            },
        })
    }

    fn row_json(&self, row: &Row) -> serde_json::Value {
        match row {
            Row::File(f) => json!({
                "kind": if f.is_dir { "folder" } else { "file" },
                "path": self.rel(&f.path),
                "non_project": f.non_project,
                "recent": f.recent,
                "weight": f.weight,
                "location": f.location,
                "package": f.package,
            }),
            Row::Action(i) => {
                let a = &self.context.actions[*i];
                json!({
                    "kind": "action",
                    "id": a.id,
                    "title": a.title.to_string(),
                    "shortcut": a.shortcut.as_ref().map(|s| s.to_string()),
                    "available": a.available,
                })
            }
            Row::Text(t) => json!({
                "kind": "text",
                "path": self.rel(&t.path),
                "line": t.line + 1,
                "text": t.text,
                "spans": t.spans.iter().map(|(r, role)| json!([r.start, r.end, role.name()])).collect::<Vec<_>>(),
            }),
            Row::More => json!({ "kind": "more" }),
        }
    }

    /// A path relative to the project, with `/`.
    pub(crate) fn rel(&self, path: &Path) -> String {
        let rel = self
            .context
            .root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        rel.to_string_lossy().replace('\\', "/")
    }

    /// Only the tabs and the field: nothing typed and nothing to list.
    fn collapsed(&self, cx: &App) -> bool {
        self.rows.is_empty() && self.field.read(cx).text().is_empty()
    }

    // ---- opening and closing ------------------------------------------------

    /// Opens the popup on `tab` (or, when it is open, switches the tab, or
    /// on the same tab switches its scope).
    pub fn show(
        &mut self,
        tab: Tab,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open {
            if tab == self.tab {
                self.toggle_everywhere(cx);
            } else {
                self.switch_tab(tab, cx);
            }
            return;
        }
        let selection = context
            .selection
            .clone()
            .filter(|s| !s.is_empty() && !s.contains('\n'))
            .filter(|_| tab != Tab::Actions);
        self.context = context;
        self.open = true;
        self.tab = tab;
        // Each tab starts with its own scope (the Files tab's is kept).
        let saved = settings(cx);
        self.everywhere = [false; 4];
        self.everywhere[Tab::Files.index()] = saved.files_all_places;
        self.chosen = [false; 4];
        self.chosen[Tab::Files.index()] = saved.files_all_places;
        self.auto_everywhere = None;
        let shared = crate::find_in_files::settings(cx);
        self.options = shared.options();
        self.mask = shared.file_mask.clone();
        self.rows.clear();
        self.selected = None;
        self.selection_moved = false;
        self.frozen = 0;
        self.history_at = None;
        self.dropdown = None;
        let text = selection.unwrap_or_else(|| self.last_search(tab));
        self.field
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        window.focus(&self.field.focus_handle(cx));
        if self.everywhere() {
            cx.emit(SearchEverywhereEvent::WantNonProject);
        }
        self.check_recent_files(cx);
        self.search_now(cx);
        cx.notify();
    }

    /// Leaves out the recent files deleted since they were opened (the
    /// disk is asked in the background; a change searches again).
    fn check_recent_files(&mut self, cx: &mut Context<Self>) {
        let files = self.context.recent_files.clone();
        let check = cx.background_spawn(async move {
            let gone: HashSet<PathBuf> = files.into_iter().filter(|p| !p.exists()).collect();
            gone
        });
        self.recent_check = Some(cx.spawn(async move |this, cx| {
            let gone = check.await;
            let _ = this.update(cx, |this, cx| {
                this.recent_check = None;
                if gone.is_empty() || !this.open {
                    return;
                }
                this.context.recent_files.retain(|p| !gone.contains(p));
                this.frozen = 0;
                this.search_now(cx);
            });
        }));
    }

    /// Opens on `tab` with `text` (the command for agents and tests).
    pub fn search_for(
        &mut self,
        tab: Tab,
        text: &str,
        non_project: Option<bool>,
        context: OpenContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            self.show(tab, context, window, cx);
        } else if self.tab != tab {
            self.switch_tab(tab, cx);
        }
        if let Some(on) = non_project {
            self.set_everywhere(on, false, cx);
        }
        self.field.update(cx, |f, cx| f.set_text_selected(text, cx));
        self.debounce = None;
        self.frozen = 0;
        self.search_now(cx);
    }

    /// Closes the popup; with `restore_focus` the focus goes back to where
    /// it was (Escape). The search is kept in the tab's history.
    pub fn close(&mut self, restore_focus: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let text = self.field.read(cx).text();
        self.remember(&text);
        self.last_selection = self
            .selected
            .and_then(|i| self.rows.get(i))
            .and_then(row_key)
            .map(|key| (self.tab, text.clone(), key));
        self.open = false;
        self.cancel_search();
        self.debounce = None;
        self.recent_check = None;
        self.dropdown = None;
        self.frame.cancel_drag();
        self.clear_preview(cx);
        let place = self.frame.place();
        update_settings(cx, |s| s.popup = place);
        if restore_focus {
            cx.emit(SearchEverywhereEvent::Dismissed);
        }
        cx.notify();
    }

    fn remember(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let tab = self.tab;
        self.history.retain(|(t, x)| !(t == text && *x == tab));
        self.history.push((text.to_owned(), tab));
        let count = self.history.iter().filter(|(_, x)| *x == tab).count();
        if count > HISTORY_LIMIT
            && let Some(i) = self.history.iter().position(|(_, x)| *x == tab)
        {
            self.history.remove(i);
        }
    }

    /// The searches of a tab, oldest first (All: every tab's, without
    /// repeats).
    pub(crate) fn tab_history(&self, tab: Tab) -> Vec<String> {
        match tab {
            Tab::All => {
                let mut out: Vec<String> = Vec::new();
                for (text, _) in self.history.iter().rev() {
                    if !out.contains(text) {
                        out.push(text.clone());
                    }
                }
                out.truncate(HISTORY_LIMIT);
                out.reverse();
                out
            }
            _ => self
                .history
                .iter()
                .filter(|(_, t)| *t == tab)
                .map(|(text, _)| text.clone())
                .collect(),
        }
    }

    fn last_search(&self, tab: Tab) -> String {
        self.tab_history(tab).pop().unwrap_or_default()
    }

    /// Alt+Up / Alt+Down: the tab's searches, wrapping around.
    fn step_history(&mut self, back: bool, cx: &mut Context<Self>) {
        let list = self.tab_history(self.tab);
        if list.is_empty() {
            return;
        }
        let n = list.len();
        let at = match (self.history_at, back) {
            (None, true) => n - 1,
            (None, false) => 0,
            (Some(i), true) => (i + n - 1) % n,
            (Some(i), false) => (i + 1) % n,
        };
        self.set_text(&list[at].clone(), cx);
        self.history_at = Some(at);
    }

    /// Puts a text in the field (selected) and searches it.
    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.field.update(cx, |f, cx| f.set_text_selected(text, cx));
        self.text_changed(cx);
    }

    // ---- tabs, scopes and options -----------------------------------------

    pub(crate) fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        // A scope the popup widened by itself goes back.
        if self.auto_everywhere.take().is_some() {
            let i = self.tab.index();
            self.everywhere[i] = self.chosen[i] && self.everywhere[i];
        }
        self.tab = tab;
        self.dropdown = None;
        self.rows.clear();
        self.frozen = 0;
        self.selected = None;
        self.selection_moved = false;
        if self.everywhere() && tab != Tab::Actions {
            cx.emit(SearchEverywhereEvent::WantNonProject);
        }
        self.search_now(cx);
        cx.notify();
    }

    fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let tabs = Tab::ALL;
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = tabs.len();
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.switch_tab(tabs[next], cx);
    }

    pub fn toggle_everywhere(&mut self, cx: &mut Context<Self>) {
        let on = !self.everywhere();
        self.set_everywhere(on, true, cx);
    }

    /// Widens or narrows the tab's scope; `chosen` when the person did (the
    /// Files tab keeps it across sessions).
    pub(crate) fn set_everywhere(&mut self, on: bool, chosen: bool, cx: &mut Context<Self>) {
        let i = self.tab.index();
        if chosen {
            self.auto_everywhere = None;
            self.chosen[i] = true;
            if self.tab == Tab::Files {
                update_settings(cx, |s| s.files_all_places = on);
            }
        }
        if self.everywhere[i] == on {
            return;
        }
        self.everywhere[i] = on;
        if on && self.tab != Tab::Actions {
            cx.emit(SearchEverywhereEvent::WantNonProject);
        }
        self.frozen = 0;
        self.search_now(cx);
        cx.notify();
    }

    /// Match Case, Words, Regex of the Text tab (kept for Find in Files).
    fn change_options(&mut self, change: impl FnOnce(&mut SearchOptions), cx: &mut Context<Self>) {
        change(&mut self.options);
        let options = self.options;
        crate::find_in_files::update_settings(cx, |s| {
            s.case_sensitive = options.case_sensitive;
            s.whole_words = options.whole_words;
            s.regex = options.regex;
        });
        if self.tab == Tab::Text {
            self.frozen = 0;
            self.search_now(cx);
        }
        cx.notify();
    }

    /// The Text tab's file mask (`None`: all file types; kept for Find in
    /// Files).
    pub(crate) fn set_mask(&mut self, mask: Option<String>, cx: &mut Context<Self>) {
        self.mask = mask.clone();
        crate::find_in_files::update_settings(cx, |s| {
            if let Some(m) = &mask {
                s.remember_mask(m);
            }
            s.file_mask = mask;
        });
        self.dropdown = None;
        self.frozen = 0;
        self.search_now(cx);
        cx.notify();
    }

    /// A filter item checked or unchecked (a file type, "Directory", or a
    /// kind of result on All); `None` checks all, `Some("")`... see callers.
    pub(crate) fn set_filter(
        &mut self,
        kind: &Dropdown,
        item: Option<&str>,
        shown: bool,
        cx: &mut Context<Self>,
    ) {
        let all: Vec<String> = match kind {
            Dropdown::FileTypes => self.filter_types(),
            _ => Provider::ALL.iter().map(|p| p.title().to_owned()).collect(),
        };
        update_settings(cx, |s| {
            let hidden = match kind {
                Dropdown::FileTypes => &mut s.hidden_file_types,
                _ => &mut s.hidden_providers,
            };
            match item {
                Some(name) => {
                    hidden.retain(|h| h != name);
                    if !shown {
                        hidden.push(name.to_owned());
                    }
                }
                None if shown => hidden.clear(),
                None => *hidden = all,
            }
        });
        self.frozen = 0;
        self.search_now(cx);
        cx.notify();
    }

    /// Inverts the checked items of a filter.
    pub(crate) fn invert_filter(&mut self, kind: &Dropdown, cx: &mut Context<Self>) {
        let all: Vec<String> = match kind {
            Dropdown::FileTypes => self.filter_types(),
            _ => Provider::ALL.iter().map(|p| p.title().to_owned()).collect(),
        };
        update_settings(cx, |s| {
            let hidden = match kind {
                Dropdown::FileTypes => &mut s.hidden_file_types,
                _ => &mut s.hidden_providers,
            };
            *hidden = all.into_iter().filter(|a| !hidden.contains(a)).collect();
        });
        self.frozen = 0;
        self.search_now(cx);
        cx.notify();
    }

    /// The items of the Files tab's filter: folders, then the file types.
    pub(crate) fn filter_types(&self) -> Vec<String> {
        let mut out = vec![DIRECTORY_TYPE.to_owned()];
        for t in self.file_types.types() {
            if !out.iter().any(|o| o == t.name) {
                out.push(t.name.to_owned());
            }
        }
        out
    }

    pub(crate) fn show_dropdown(&mut self, dropdown: Dropdown, cx: &mut Context<Self>) {
        self.dropdown = if self.dropdown.as_ref() == Some(&dropdown) {
            None
        } else {
            Some(dropdown)
        };
        cx.notify();
    }

    /// The index changed (built, rebuilt, non-project items listed).
    pub fn set_index(&mut self, index: Option<Arc<FileIndex>>, cx: &mut Context<Self>) {
        let same = match (&self.context.index, &index) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.context.index = index;
        if self.open {
            self.frozen = 0;
            self.search_now(cx);
        }
    }

    // ---- searching ----------------------------------------------------------

    fn text_changed(&mut self, cx: &mut Context<Self>) {
        self.selection_moved = false;
        self.cancel_search();
        let text = self.field.read(cx).text();
        // A scope widened because nothing was found goes back unless the
        // text still contains what found nothing (longer than one char).
        if let Some(not_found) = &self.auto_everywhere
            && !(not_found.chars().count() > 1 && text.contains(not_found.as_str()))
        {
            self.auto_everywhere = None;
            self.everywhere[self.tab.index()] = false;
        }
        self.frozen = 0;
        if text.is_empty() {
            self.debounce = None;
            self.search_now(cx);
            return;
        }
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.debounce = None;
                this.search_now(cx);
            });
        }));
        cx.notify();
    }

    fn limit(&self, provider: Provider) -> usize {
        self.limits.get(&provider).copied().unwrap_or(self.page())
    }

    /// Rows of each kind a page shows on this tab.
    fn page(&self) -> usize {
        match self.tab {
            Tab::All | Tab::Files => PAGE,
            Tab::Actions | Tab::Text => SINGLE_PAGE,
        }
    }

    fn cancel_search(&mut self) {
        self.search_task = None;
        self.searching = false;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    /// The providers the tab asks, without those the All tab's filter
    /// leaves out.
    fn providers(&self, cx: &App) -> Vec<Provider> {
        let hidden = settings(cx).hidden_providers;
        self.tab
            .providers()
            .iter()
            .copied()
            .filter(|p| self.tab != Tab::All || !hidden.iter().any(|h| h == p.title()))
            .collect()
    }

    /// Searches the tab now: recent files and actions here, files and text
    /// in the background.
    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        if self.frozen == 0 {
            self.limits.clear();
        }
        self.generation += 1;
        let generation = self.generation;
        self.searching = true;
        self.started = Some(std::time::Instant::now());
        let text = self.field.read(cx).text();
        let providers = self.providers(cx);
        let everywhere = self.everywhere();
        let mut found = Vec::new();
        for &p in &providers {
            match p {
                Provider::Recent => found.push(providers::recent(
                    &self.context.recent_files,
                    self.context.current_file.as_ref(),
                    &text,
                    self.limit(p),
                    &self.places(),
                )),
                Provider::Actions => found.push(providers::actions(
                    &self.context.actions,
                    &text,
                    everywhere && self.tab == Tab::Actions,
                    self.limit(p),
                )),
                Provider::Files | Provider::Text => {}
            }
        }
        let blank = text.trim().is_empty();
        let files = (providers.contains(&Provider::Files) && !blank)
            .then(|| self.files_query(&text, cx))
            .flatten();
        let text_search = (providers.contains(&Provider::Text) && !blank)
            .then(|| self.text_search(&text))
            .flatten();
        if files.is_none() && text_search.is_none() {
            self.show_found(found, &text, cx);
            return;
        }
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        let types = (*self.file_types).clone();
        let task = cx.background_spawn(async move {
            let mut more = Vec::new();
            if let Some(q) = files {
                more.push(providers::files(q, &cancel));
            }
            if let Some(search) = text_search {
                more.push(providers::text(search, &types, &cancel));
            }
            more
        });
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let more = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                found.extend(more);
                this.show_found(found, &text, cx);
            });
        }));
        cx.notify();
    }

    fn files_query(&self, text: &str, cx: &App) -> Option<providers::FilesQuery> {
        let index = self.context.index.clone()?;
        let hidden = if self.tab == Tab::Files {
            settings(cx).hidden_file_types.into_iter().collect()
        } else {
            HashSet::new()
        };
        Some(providers::FilesQuery {
            index,
            text: text.to_owned(),
            non_project: self.everywhere(),
            open_files: self.context.open_files.clone(),
            current: self.context.current_file.clone(),
            home: self.context.home.clone(),
            hidden_types: hidden,
            types: (*self.file_types).clone(),
            limit: self.limit(Provider::Files),
            places: self.places(),
        })
    }

    /// The footer's text for the selected row (its path from its source
    /// root, or an action's description), and whether it offers "Open In
    /// Right Split" (a file).
    pub(crate) fn footer(&self) -> (String, bool) {
        match self.selected.and_then(|i| self.rows.get(i)) {
            Some(Row::File(f)) => (self.places().footer_path(&f.path), !f.is_dir),
            Some(Row::Text(t)) => (self.places().footer_path(&t.path), true),
            Some(Row::Action(i)) => (self.context.actions[*i].description.to_string(), false),
            _ => (String::new(), false),
        }
    }

    /// Where rows are shown from (locations, the footer).
    fn places(&self) -> providers::Places {
        providers::Places {
            root: self.context.root.clone(),
            source_roots: self.context.source_roots.clone(),
            hooks: self.context.navigation,
            home: self.context.home.clone(),
        }
    }

    /// The Text tab's search (on All: plain, no mask).
    fn text_search(&self, text: &str) -> Option<FileSearch> {
        let root = self.context.root.clone()?;
        let (options, mask) = if self.tab == Tab::Text {
            let mask = match &self.mask {
                Some(m) => Some(FileMask::parse(m).ok()?),
                None => None,
            };
            (self.options, mask)
        } else {
            (SearchOptions::default(), None)
        };
        let matcher = Query::new(text, options).compile().ok()?;
        // All Places: the excluded folders too.
        let filter = if self.everywhere() {
            Some(project::Filter::new(
                &root,
                project::Exclusions::default(),
                project::AutoExclusions::NONE,
            ))
        } else {
            self.context.filter.clone()
        };
        Some(FileSearch {
            matcher,
            mask,
            scope: Scope::Directory {
                path: root.clone(),
                recursive: true,
            },
            root,
            filter,
            limit: self.limit(Provider::Text),
            known: Default::default(),
            overrides: self.context.overrides.clone(),
            project_files: false,
        })
    }

    /// Puts the rows found in the list: recent files first (on All), then
    /// by weight, the rows that came first first between equal weights;
    /// rows shown before loading more stay where they were; a spinner row
    /// ends the list while there are more.
    /// The rows found for `text`.
    fn show_found(&mut self, found: Vec<Found>, text: &str, cx: &mut Context<Self>) {
        self.searching = false;
        self.search_task = None;
        self.cancel = None;
        self.finished = self.started.map(|t| t.elapsed());
        let text = text.to_owned();
        let previous = self.selected.and_then(|i| self.rows.get(i)).cloned();
        self.more = found
            .iter()
            .filter(|f| f.more)
            .map(|f| f.provider)
            .collect();
        let mut items: Vec<(Provider, Row, i32)> = Vec::new();
        for f in &found {
            for (row, weight) in &f.rows {
                // A file shown as a recent file is not repeated.
                if let Row::File(file) = row
                    && !file.recent
                    && items
                        .iter()
                        .any(|(_, r, _)| matches!(r, Row::File(x) if x.path == file.path))
                {
                    continue;
                }
                items.push((f.provider, row.clone(), *weight));
            }
        }
        items.sort_by(|a, b| b.0.pinned().cmp(&a.0.pinned()).then(b.2.cmp(&a.2)));
        let mut rows: Vec<Row> = self.rows.iter().take(self.frozen).cloned().collect();
        for (_, row, _) in items {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        if !self.more.is_empty() {
            rows.push(Row::More);
        }
        let nothing = rows.is_empty();
        let new_from = self.frozen;
        self.rows = rows;
        // Nothing found: the tab's scope widens by itself (unless the
        // person set it in this popup).
        let i = self.tab.index();
        if nothing && !text.trim().is_empty() && !self.everywhere[i] && !self.chosen[i] {
            self.everywhere[i] = true;
            self.auto_everywhere = Some(text.clone());
            if self.tab != Tab::Actions {
                cx.emit(SearchEverywhereEvent::WantNonProject);
            }
            self.search_now(cx);
            return;
        }
        let real = self.rows.len();
        self.selected = if real == 0 {
            None
        } else if new_from > 0 {
            // The next page: its first row, where "… more" was.
            Some(new_from.min(real - 1))
        } else if self.selection_moved {
            previous_index(&self.rows, previous.as_ref()).or(Some(0))
        } else {
            self.restored_selection(&text).or(Some(0))
        };
        self.preview_selected(cx);
        cx.notify();
    }

    /// The row selected when the popup closed with this tab and text.
    fn restored_selection(&mut self, text: &str) -> Option<usize> {
        let (tab, last_text, key) = self.last_selection.take()?;
        if tab != self.tab || last_text != text {
            return None;
        }
        self.rows
            .iter()
            .position(|r| row_key(r).as_ref() == Some(&key))
    }

    /// "… more": every kind that has more shows a page more; the rows
    /// shown stay where they are.
    pub(crate) fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.more.is_empty() || self.is_searching() {
            return;
        }
        self.rows.retain(|r| *r != Row::More);
        self.frozen = self.rows.len();
        let page = self.page();
        for p in self.more.clone() {
            let limit = self.limit(p) + page;
            self.limits.insert(p, limit);
        }
        self.search_now(cx);
    }

    // ---- moving and choosing ----------------------------------------------

    /// Up and Down; past the ends round to the other end (Up only once
    /// the search is over, as more rows may still come).
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None => 0,
            Some(0) if delta < 0 && self.is_searching() => 0,
            Some(i) => (i as isize + delta).rem_euclid(n as isize) as usize,
        };
        self.select_row(next, cx);
    }

    /// Rows the list shows at once.
    fn visible_rows(&self, cx: &App) -> usize {
        let height = self.list.0.borrow().base_handle.bounds().size.height;
        let row = cx.theme().metric(themes::Metric::RowHeight);
        if row <= px(0.0) {
            return 1;
        }
        (f32::from(height) / f32::from(row)).floor().max(1.0) as usize
    }

    pub(crate) fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.rows.len() {
            return;
        }
        self.selected = Some(index);
        self.selection_moved = true;
        // As little scrolling as shows the row.
        let row_top = cx.theme().metric(themes::Metric::RowHeight) * index as f32;
        let scroll_top = -self.list.0.borrow().base_handle.offset().y;
        let strategy = if row_top < scroll_top {
            gpui::ScrollStrategy::Top
        } else {
            gpui::ScrollStrategy::Bottom
        };
        self.list.scroll_to_item(index, strategy);
        self.preview_selected(cx);
        cx.notify();
    }

    /// Enter: opens the file or line, or runs the action.
    pub fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        match row {
            Row::More => self.load_more(cx),
            Row::File(file) => {
                let text = self.field.read(cx).text();
                let position = parse_position(&text);
                self.close(false, window, cx);
                if file.is_dir {
                    cx.emit(SearchEverywhereEvent::RevealFolder(file.path));
                } else {
                    cx.emit(SearchEverywhereEvent::OpenFile {
                        path: file.path,
                        line: position.line,
                        column: position.column,
                    });
                }
            }
            Row::Text(line) => {
                self.close(false, window, cx);
                cx.emit(SearchEverywhereEvent::OpenFile {
                    path: line.path,
                    line: Some(line.line),
                    column: Some(line.column),
                });
            }
            Row::Action(i) => {
                let id = self.context.actions[i].id;
                self.close(true, window, cx);
                cx.emit(SearchEverywhereEvent::Run(id));
            }
        }
    }

    // ---- actions ----------------------------------------------------------

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if self.dropdown.take().is_some() {
            cx.notify();
            return;
        }
        self.close(true, window, cx);
    }

    fn next_tab_action(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(true, cx);
    }

    fn previous_tab_action(&mut self, _: &PreviousTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(false, cx);
    }

    fn down_action(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn up_action(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    /// Page Down: a page further, keeping a few rows of the last page.
    fn next_group_action(&mut self, _: &NextGroup, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let step = self.visible_rows(cx).saturating_sub(PAGE_OVERLAP).max(1);
        let next = self.selected.map_or(0, |i| i + step).min(n - 1);
        self.select_row(next, cx);
    }

    fn previous_group_action(&mut self, _: &PreviousGroup, _: &mut Window, cx: &mut Context<Self>) {
        self.select_row(0, cx);
    }

    fn select_action(&mut self, _: &Select, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected {
            self.choose(i, window, cx);
        }
    }

    /// The scope list's mnemonic: on the Files and Text tabs.
    fn choose_scope_action(&mut self, _: &ChooseScope, _: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.tab, Tab::Files | Tab::Text) {
            cx.propagate();
            return;
        }
        self.show_dropdown(Dropdown::Scope, cx);
    }

    fn toggle_everywhere_action(
        &mut self,
        _: &ToggleEverywhere,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_everywhere(cx);
    }

    fn history_previous_action(
        &mut self,
        _: &HistoryPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_history(true, cx);
    }

    fn history_next_action(&mut self, _: &HistoryNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, cx);
    }

    /// The Text tab's options; elsewhere the key goes on to the field.
    fn toggle_case_action(&mut self, _: &ToggleCase, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab != Tab::Text {
            cx.propagate();
            return;
        }
        self.change_options(|o| o.case_sensitive = !o.case_sensitive, cx);
    }

    fn toggle_words_action(&mut self, _: &ToggleWords, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab != Tab::Text {
            cx.propagate();
            return;
        }
        self.change_options(|o| o.whole_words = !o.whole_words, cx);
    }

    fn toggle_preview_action(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab != Tab::Actions {
            self.toggle_preview(cx);
        }
    }

    fn toggle_regex_action(&mut self, _: &ToggleRegex, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab != Tab::Text {
            cx.propagate();
            return;
        }
        self.change_options(|o| o.regex = !o.regex, cx);
    }
}

/// How a row is found again after a reopen.
fn row_key(row: &Row) -> Option<RowKey> {
    match row {
        Row::File(f) => Some(RowKey::Path(f.path.clone())),
        Row::Action(_) => None,
        Row::Text(t) => Some(RowKey::Line(t.path.clone(), t.line)),
        Row::More => None,
    }
}

fn previous_index(rows: &[Row], previous: Option<&Row>) -> Option<usize> {
    let previous = previous?;
    rows.iter().position(|r| r == previous && *r != Row::More)
}

/// Registers the commands.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
}

macro_rules! popup_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl SearchEverywhere {
            fn bind_actions<E: InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }
    };
}

popup_actions! {
    Close => close_action,
    NextTab => next_tab_action,
    PreviousTab => previous_tab_action,
    Down => down_action,
    Up => up_action,
    NextGroup => next_group_action,
    PreviousGroup => previous_group_action,
    Select => select_action,
    ChooseScope => choose_scope_action,
    ToggleEverywhere => toggle_everywhere_action,
    HistoryPrevious => history_previous_action,
    HistoryNext => history_next_action,
    ToggleCase => toggle_case_action,
    ToggleWords => toggle_words_action,
    ToggleRegex => toggle_regex_action,
    TogglePreview => toggle_preview_action,
}
