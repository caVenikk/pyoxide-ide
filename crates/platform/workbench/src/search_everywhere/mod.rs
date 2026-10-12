//! Search Everywhere and Go to File: one popup with tabs (All, Files,
//! Actions), as in the reference IDE (tests/scenarios/editor/search-everywhere.md).
//!
//! Double Shift opens it on All, Go to File on Files, Go to Action on
//! Actions; pressing the shortcut of the tab shown again switches
//! "Include non-project items" (on Actions: disabled actions) on or off,
//! another one switches the tab. The field starts with the text selected
//! in the editor, else the tab's last search, selected.
//!
//! - Files: the project's file name index ([`crate::project_files`]) finds
//!   files and folders as the reference ranks them (`file-index`);
//!   `name:line:column` opens at that place.
//! - Actions: the registered commands by title ([`actions`]).
//! - All: recently opened files that match on top, then files and
//!   actions by weight; with an empty field the recent files.
//!
//! A tab shows up to [`SINGLE_LIMIT`] results of each kind (All:
//! [`MULTIPLE_LIMIT`]) and a "… more" row for the next ones. Typing
//! searches again [`DEBOUNCE`] after the last key (at once for an empty
//! field); a search that finds nothing in the project looks at non-project
//! items too, until the text no longer contains the text that found
//! nothing.

pub mod actions;
mod settings;
mod view;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use editor::Editor;
use file_index::{FileHit, FileIndex, GotoQuery, parse_position, rank};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, Subscription, Task, UniformListScrollHandle, Window, prelude::*, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::json;
use text::Buffer;

use ui::theme::ThemeExt as _;

use crate::file_types::FileTypes;
use crate::popup::PopupFrame;
use actions::ActionItem;
use settings::update_settings;
pub use settings::{SearchEverywhereSettings, load_settings, settings, settings_file};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Search Everywhere (All tab).
    Open => "search-everywhere.open", "Search Everywhere",
        "Opens Search Everywhere on the All tab: recent files, files and actions.";
    /// Go to File.
    GoToFile => "search-everywhere.go-to-file", "Go to File…",
        "Opens Search Everywhere on the Files tab to open a file by its name (`name:line:column` opens at a line).";
    /// Go to Action.
    GoToAction => "search-everywhere.go-to-action", "Find Action…",
        "Opens Search Everywhere on the Actions tab to run a command by its name.";
    /// Search for a text in a tab.
    Search {
        text: String,
        tab: Option<String>,
        non_project: Option<bool>,
    } => "search-everywhere.search", "Search Everywhere for Text",
        "Opens Search Everywhere on a tab (`all`, `files`, `actions`) with a text; `non_project` includes non-project items.",
        r#"{"text": "app.py:12", "tab": "files", "non_project": false}"#;
    /// Describe the popup.
    State => "search-everywhere.state", "Search Everywhere State",
        "Returns the state of the Search Everywhere popup as JSON (tab, text, rows, selected row).";
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
    Up => "search-everywhere.up", "Previous Result", "Selects the previous result (before the first, the last).";
    /// The last row.
    NextGroup => "search-everywhere.next-group", "Last Result", "Selects the last result.";
    /// The first row.
    PreviousGroup => "search-everywhere.previous-group", "First Result", "Selects the first result.";
    /// Open or run the selected row.
    Select => "search-everywhere.select", "Open Result",
        "Opens the selected file (at the line and column typed after its name), runs the selected action, or loads more results.";
    /// Include non-project items.
    ToggleEverywhere => "search-everywhere.toggle-everywhere", "Include Non-Project Items",
        "Switches including non-project items (on the Actions tab: disabled actions) on or off.";
    /// The previous search of the tab.
    HistoryPrevious => "search-everywhere.history-previous", "Previous Search",
        "Puts the tab's previous search in the field.";
    /// The next search of the tab.
    HistoryNext => "search-everywhere.history-next", "Next Search",
        "Puts the tab's next search in the field.";
}

/// Results of each kind on a tab of one kind, and loaded by "more".
pub const SINGLE_LIMIT: usize = 30;
/// Results of each kind on the All tab, and loaded by "more".
pub const MULTIPLE_LIMIT: usize = 15;
/// Wait after a key before searching (the reference's).
pub const DEBOUNCE: Duration = Duration::from_millis(100);
/// Searches kept per tab.
const HISTORY_LIMIT: usize = 50;
/// Default size (the reference's minimum width and list height); the
/// popup is centred and a quarter down the window.
const WIDTH: f32 = 700.0;
const HEIGHT: f32 = 600.0;
const MIN_WIDTH: f32 = 700.0;
/// Rows a page down leaves of the previous page.
const PAGE_OVERLAP: usize = 4;
/// The header with the tabs, and the field.
const HEADER_HEIGHT: f32 = 40.0;
const FIELD_HEIGHT: f32 = 40.0;
/// The list is at least this high when shown.
const MIN_LIST_HEIGHT: f32 = 100.0;

/// The tabs, in the reference's order (it shows Classes and Symbols when a
/// language provides them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    All,
    Files,
    Actions,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::All, Tab::Files, Tab::Actions];

    pub fn title(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Files => "Files",
            Tab::Actions => "Actions",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tab::All => "all",
            Tab::Files => "files",
            Tab::Actions => "actions",
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
        }
    }

    /// Results of each kind.
    fn limit(self) -> usize {
        match self {
            Tab::All => MULTIPLE_LIMIT,
            _ => SINGLE_LIMIT,
        }
    }
}

/// What the popup opens with (the workbench's state).
#[derive(Default, Clone)]
pub struct OpenContext {
    pub root: Option<PathBuf>,
    pub index: Option<Arc<FileIndex>>,
    /// The editor's selection (one line), for the field.
    pub selection: Option<String>,
    pub open_files: Vec<PathBuf>,
    /// The file of the active editor.
    pub current_file: Option<PathBuf>,
    /// Files opened in editors, most recent first.
    pub recent_files: Vec<PathBuf>,
    /// The commands, as available where the focus was.
    pub actions: Vec<ActionItem>,
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

/// A row of the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    File(FileRow),
    Action(usize),
    /// "… more": the next results.
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
}

/// Which provider a row came from, for the order of the All tab and for
/// loading more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Provider {
    Recent,
    Files,
    Actions,
}

impl Provider {
    /// Lower first between equal weights (the reference's sort weights).
    fn sort_weight(self) -> i32 {
        match self {
            Provider::Recent => 70,
            Provider::Files => 200,
            Provider::Actions => 400,
        }
    }
}

/// What a search found for one provider: rows in order, and whether there
/// are more.
struct Found {
    provider: Provider,
    rows: Vec<(Row, i32)>,
    more: bool,
}

pub struct SearchEverywhere {
    focus: FocusHandle,
    field: Entity<Editor>,
    file_types: Rc<FileTypes>,
    open: bool,
    tab: Tab,
    context: OpenContext,
    /// "Include non-project items" (All, Files) and "Include disabled
    /// actions" (Actions).
    everywhere: bool,
    /// Switched on by the popup because nothing was found (the text that
    /// found nothing), not by the person.
    auto_everywhere: Option<String>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// Rows of each provider asked for (grows with "more").
    limits: Vec<(Provider, usize)>,
    /// Rows shown before "more" was chosen stay where they are.
    frozen: usize,
    searching: bool,
    generation: u64,
    search_task: Option<Task<()>>,
    debounce: Option<Task<()>>,
    /// Searches per tab, oldest first, and the position while going
    /// through them.
    history: Vec<(String, Tab)>,
    history_at: Option<usize>,
    list: UniformListScrollHandle,
    frame: PopupFrame,
    /// Measurements: when the last search started and how long it took.
    started: Option<std::time::Instant>,
    finished: Option<Duration>,
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
        Self {
            focus,
            field,
            file_types,
            open: false,
            tab: Tab::All,
            context: OpenContext::default(),
            everywhere: false,
            auto_everywhere: None,
            rows: Vec::new(),
            selected: None,
            limits: Vec::new(),
            frozen: 0,
            searching: false,
            generation: 0,
            search_task: None,
            debounce: None,
            history: Vec::new(),
            history_at: None,
            list: UniformListScrollHandle::new(),
            frame,
            started: None,
            finished: None,
            _subscriptions: subscriptions,
        }
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

    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let rows: Vec<_> = self.rows.iter().map(|r| self.row_json(r)).collect();
        json!({
            "open": self.open,
            "tab": self.tab.name(),
            "tabs": Tab::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
            "text": self.field.read(cx).text(),
            "everywhere": self.everywhere,
            "searching": self.is_searching(),
            "rows": rows,
            "selected": self.selected,
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

    // ---- opening and closing ------------------------------------------------

    /// Opens the popup on `tab` (or, when it is open, switches the tab, or
    /// on the same tab switches non-project items).
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
        self.everywhere = false;
        self.auto_everywhere = None;
        self.rows.clear();
        self.selected = None;
        self.frozen = 0;
        self.history_at = None;
        let text = selection.unwrap_or_else(|| self.last_search(tab));
        self.field
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        window.focus(&self.field.focus_handle(cx));
        self.search_now(cx);
        cx.notify();
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
            self.set_everywhere(on, cx);
        }
        self.field.update(cx, |f, cx| f.set_text_selected(text, cx));
        self.debounce = None;
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
        self.open = false;
        self.search_task = None;
        self.debounce = None;
        self.searching = false;
        self.frame.cancel_drag();
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
    fn tab_history(&self, tab: Tab) -> Vec<String> {
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
        let text = list[at].clone();
        self.field
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        self.text_changed(cx);
        self.history_at = Some(at);
    }

    // ---- tabs and options -------------------------------------------------

    fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        // Each tab starts with project items only, unless switched on by
        // the person in this popup.
        if self.auto_everywhere.is_some() || !self.everywhere_chosen() {
            self.everywhere = false;
            self.auto_everywhere = None;
        }
        self.frozen = 0;
        self.selected = None;
        self.search_now(cx);
        cx.notify();
    }

    /// The person switched non-project items on in this popup.
    fn everywhere_chosen(&self) -> bool {
        self.everywhere && self.auto_everywhere.is_none()
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
        let on = !self.everywhere;
        self.set_everywhere(on, cx);
    }

    fn set_everywhere(&mut self, on: bool, cx: &mut Context<Self>) {
        self.auto_everywhere = None;
        if self.everywhere == on {
            return;
        }
        self.everywhere = on;
        if on && self.tab != Tab::Actions {
            cx.emit(SearchEverywhereEvent::WantNonProject);
        }
        self.frozen = 0;
        self.search_now(cx);
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
            self.search_now(cx);
        }
    }

    // ---- searching ----------------------------------------------------------

    fn text_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text();
        // Non-project items switched on because nothing was found go off
        // when the text no longer contains what found nothing.
        if let Some(not_found) = &self.auto_everywhere
            && not_found.chars().count() > 1
            && !text.contains(not_found.as_str())
        {
            self.auto_everywhere = None;
            self.everywhere = false;
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

    fn reset_limits(&mut self) {
        self.limits.clear();
    }

    fn limit(&self, provider: Provider) -> usize {
        self.limits
            .iter()
            .find(|(p, _)| *p == provider)
            .map_or(self.tab.limit(), |(_, l)| *l)
    }

    /// Searches the tab now (in the background for files).
    fn search_now(&mut self, cx: &mut Context<Self>) {
        if self.frozen == 0 {
            self.reset_limits();
        }
        self.generation += 1;
        let generation = self.generation;
        self.searching = true;
        self.started = Some(std::time::Instant::now());
        let text = self.field.read(cx).text();
        let tab = self.tab;
        let providers: &[Provider] = match tab {
            Tab::All => &[Provider::Recent, Provider::Files, Provider::Actions],
            Tab::Files => &[Provider::Files],
            Tab::Actions => &[Provider::Actions],
        };
        // Recent files and actions are found here; files in the background.
        let mut found = Vec::new();
        for &p in providers {
            match p {
                Provider::Recent => found.push(self.find_recent(&text)),
                Provider::Actions => found.push(self.find_actions(&text)),
                Provider::Files => {}
            }
        }
        let files_wanted = providers.contains(&Provider::Files) && !text.trim().is_empty();
        let index = self.context.index.clone().filter(|_| files_wanted);
        let Some(index) = index else {
            self.show_found(found, cx);
            return;
        };
        let limit = self.limit(Provider::Files);
        let query_text = text.clone();
        let non_project = self.everywhere && tab != Tab::Actions;
        let open_files = self.context.open_files.clone();
        let current = self.context.current_file.clone();
        let task = cx.background_spawn(async move {
            let query = GotoQuery {
                text: &query_text,
                non_project,
                open_files: &open_files,
                context: current.as_deref(),
            };
            let hits = index.goto_file(&query, &|| false);
            let more = hits.len() > limit;
            let mut page: Vec<FileHit> = hits.into_iter().take(limit).collect();
            rank(&mut page);
            let rows: Vec<(Row, i32)> = page
                .into_iter()
                .map(|h| {
                    (
                        Row::File(FileRow {
                            path: index.path(h.id),
                            is_dir: h.is_dir,
                            non_project: h.non_project,
                            recent: false,
                            weight: h.weight,
                        }),
                        h.weight,
                    )
                })
                .collect();
            Found {
                provider: Provider::Files,
                rows,
                more,
            }
        });
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let files = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                found.push(files);
                this.show_found(found, cx);
            });
        }));
        cx.notify();
    }

    /// Recent files whose names match, best first (all of them for an
    /// empty field); the file of the active editor is left out.
    fn find_recent(&self, text: &str) -> Found {
        let pattern = parse_position(text).pattern;
        let pattern = pattern.trim();
        let mut matcher = Matcher::new(&format!("*{pattern}"), MatchingMode::IgnoreCase);
        if !pattern.starts_with('*') {
            matcher = matcher.prefer_start();
        }
        let mut rows: Vec<(Row, i32)> = Vec::new();
        for path in &self.context.recent_files {
            if Some(path) == self.context.current_file.as_ref() || !path.exists() {
                continue;
            }
            if rows
                .iter()
                .any(|(r, _)| matches!(r, Row::File(f) if &f.path == path))
            {
                continue;
            }
            let name: Vec<char> = path
                .file_name()
                .map(|n| n.to_string_lossy().chars().collect())
                .unwrap_or_default();
            let Some(m) = matcher.match_chars(&name) else {
                continue;
            };
            rows.push((
                Row::File(FileRow {
                    path: path.clone(),
                    is_dir: false,
                    non_project: false,
                    recent: true,
                    weight: m.degree,
                }),
                m.degree,
            ));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        let limit = self.limit(Provider::Recent);
        let more = rows.len() > limit;
        rows.truncate(limit);
        Found {
            provider: Provider::Recent,
            rows,
            more,
        }
    }

    fn find_actions(&self, text: &str) -> Found {
        let hits = actions::search(
            &self.context.actions,
            text,
            self.everywhere && self.tab == Tab::Actions,
        );
        let limit = self.limit(Provider::Actions);
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

    /// Puts the rows found in the list: recent files first (on All), then
    /// by weight, the providers in their order between equal weights; rows
    /// shown before "more" stay where they were; a "more" row ends the list
    /// when a provider has more.
    fn show_found(&mut self, found: Vec<Found>, cx: &mut Context<Self>) {
        self.searching = false;
        self.search_task = None;
        self.finished = self.started.map(|t| t.elapsed());
        let text = self.field.read(cx).text();
        let previous = self.selected.and_then(|i| self.rows.get(i)).cloned();
        let more = found.iter().any(|f| f.more);
        let mut items: Vec<(Provider, Row, i32)> = Vec::new();
        for f in &found {
            for (row, weight) in &f.rows {
                // A file already shown as a recent file is not repeated.
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
        items.sort_by(|a, b| {
            let top = |p: Provider| i32::from(p == Provider::Recent);
            top(b.0)
                .cmp(&top(a.0))
                .then(b.2.cmp(&a.2))
                .then(a.0.sort_weight().cmp(&b.0.sort_weight()))
        });
        let mut rows: Vec<Row> = Vec::new();
        // Rows shown before "more" keep their places.
        let frozen: Vec<Row> = self.rows.iter().take(self.frozen).cloned().collect();
        rows.extend(frozen.iter().cloned());
        for (_, row, _) in items {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        if more {
            rows.push(Row::More);
        }
        let nothing = rows.is_empty();
        self.rows = rows;
        // Nothing in the project: look at non-project items too.
        if nothing && !text.trim().is_empty() && !self.everywhere {
            self.everywhere = true;
            self.auto_everywhere = Some(text.clone());
            if self.tab != Tab::Actions {
                cx.emit(SearchEverywhereEvent::WantNonProject);
            }
            self.search_now(cx);
            return;
        }
        self.selected = if self.rows.is_empty() {
            None
        } else if self.frozen > 0 {
            Some(self.frozen.min(self.rows.len() - 1))
        } else {
            previous
                .and_then(|p| self.rows.iter().position(|r| *r == p))
                .or(Some(0))
        };
        if let Some(i) = self.selected {
            self.list.scroll_to_item(i, gpui::ScrollStrategy::Top);
        }
        cx.notify();
    }

    // ---- moving and choosing ----------------------------------------------

    /// Up and Down; past the ends round to the other end (Up only once
    /// the search is complete, as more rows may still come).
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None => 0,
            Some(i) if delta < 0 && i == 0 && self.is_searching() => 0,
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

    fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.rows.len() {
            return;
        }
        self.selected = Some(index);
        // As little scrolling as shows the row.
        let row_top = cx.theme().metric(themes::Metric::RowHeight) * index as f32;
        let scroll_top = -self.list.0.borrow().base_handle.offset().y;
        let strategy = if row_top < scroll_top {
            gpui::ScrollStrategy::Top
        } else {
            gpui::ScrollStrategy::Bottom
        };
        self.list.scroll_to_item(index, strategy);
        cx.notify();
    }

    /// Enter: opens the file, runs the action, or loads more.
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
            Row::Action(i) => {
                let id = self.context.actions[i].id;
                self.close(true, window, cx);
                cx.emit(SearchEverywhereEvent::Run(id));
            }
        }
    }

    /// "… more": every provider that has more shows that many more rows;
    /// the rows shown stay where they are.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.rows.iter().position(|r| *r == Row::More) else {
            return;
        };
        self.rows.remove(i);
        self.frozen = self.rows.len();
        let step = self.tab.limit();
        let providers = [Provider::Recent, Provider::Files, Provider::Actions];
        for p in providers {
            let shown = self
                .rows
                .iter()
                .filter(|r| match (p, r) {
                    (Provider::Recent, Row::File(f)) => f.recent,
                    (Provider::Files, Row::File(f)) => !f.recent,
                    (Provider::Actions, Row::Action(_)) => true,
                    _ => false,
                })
                .count();
            let limit = shown + step;
            match self.limits.iter_mut().find(|(x, _)| *x == p) {
                Some(l) => l.1 = limit,
                None => self.limits.push((p, limit)),
            }
        }
        self.search_now(cx);
    }

    // ---- actions ----------------------------------------------------------

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.rows.is_empty() {
            return;
        }
        let step = self.visible_rows(cx).saturating_sub(PAGE_OVERLAP).max(1);
        let next = self
            .selected
            .map_or(0, |i| i + step)
            .min(self.rows.len() - 1);
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
    ToggleEverywhere => toggle_everywhere_action,
    HistoryPrevious => history_previous_action,
    HistoryNext => history_next_action,
}
