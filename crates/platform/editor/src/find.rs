//! The find bar of an editor (Find and Replace in the file): a search field
//! with the Match Case, Words and Regex options, the match counter, the
//! previous and next buttons and the In Selection filter, and in replace
//! mode a replace field with Replace, Replace All and Exclude; shown at the
//! top of the editor.
//!
//! Matches are found by the `search` crate on a snapshot of the editor's
//! text off the UI thread; every change of the query starts a new search
//! and cancels the one still running. The editor paints the matches
//! ([`SearchHighlights`]); the current match is selected. The behaviour
//! follows the reference IDE (tests/scenarios/editor/find-replace.md): the
//! selected text or the last search fills the field, typing searches
//! incrementally, Enter and Shift+Enter move through the matches, moving
//! past the last match first says so and then starts over, Escape closes
//! the bar and keeps the current match selected, Replace All is one undo
//! step.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Task, Window, deferred, div, prelude::*, px, relative,
};
use search::{
    CancelToken, Cancelled, Matcher, Query, QueryError, ReplaceError, SearchOptions, match_lines,
    to_byte_ranges, to_char_ranges,
};
use serde_json::json;
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Selection, SelectionSet};
use themes::UiRole;
use ui::components::{
    SMALL_BUTTON, Tooltip, input_frame, small_icon_button, status_text, text_button, toggle_button,
};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::editor::{Editor, EditorEvent, SearchHighlights};

ui::ui_commands! {
    COMMANDS, scope = "Editor";
    /// Open the find bar.
    Open => "find.open", "Find…",
        "Opens the find bar of the active editor with the selected text, or the last search, selected in its field.";
    /// Open the find bar with the replace field.
    OpenReplace => "find.replace", "Replace…",
        "Opens the find bar with the replace field; a selection over several lines is searched only within.";
    /// Go to the next match.
    Next => "find.next", "Find Next Occurrence",
        "Selects the next match; past the last one it says so first and starts from the top on the next try.";
    /// Go to the previous match.
    Previous => "find.previous", "Find Previous Occurrence",
        "Selects the previous match; before the first one it says so first and starts from the bottom on the next try.";
    /// Search for a text.
    Search {
        text: String,
        case_sensitive: Option<bool>,
        whole_words: Option<bool>,
        regex: Option<bool>,
        replace: Option<String>,
    } => "find.search", "Find Text",
        "Opens the find bar and searches for a text; options that are given are switched on or off, a replacement opens the replace field.",
        r#"{"text": "def", "case_sensitive": false, "whole_words": false, "regex": false}"#;
}

ui::ui_commands! {
    BAR_COMMANDS, scope = "FindBar";
    /// Close the find bar.
    Close => "find.close", "Close",
        "Closes the find bar, removes the highlighting of the matches and keeps the current match selected.";
    /// Match case on or off.
    ToggleCase => "find.toggle-case", "Match Case", "Switches case-sensitive matching on or off (the find bar, Find in Files).";
    /// Whole words on or off.
    ToggleWords => "find.toggle-words", "Words", "Switches whole-word matching on or off (the find bar, Find in Files).";
    /// Regular expressions on or off.
    ToggleRegex => "find.toggle-regex", "Regex", "Switches regular expression matching on or off (the find bar, Find in Files).";
    /// Search within the selection.
    ToggleInSelection => "find.toggle-in-selection", "In Selection",
        "Searches only within the selection the find bar opened with, or in the whole file again.";
    /// Replace the current match.
    ReplaceCurrent => "find.replace-current", "Replace",
        "Replaces the current match and selects the next one.";
    /// Replace every match.
    ReplaceAll => "find.replace-all", "Replace All",
        "Replaces every match that is not excluded, as one undo step.";
    /// Leave the current match out of Replace All.
    Exclude => "find.exclude", "Exclude",
        "Leaves the current match out of Replace All (or takes it back) and goes to the next one.";
    /// Leave the find bar for the editor.
    FocusEditor => "find.focus-editor", "Go to Editor",
        "Moves the focus to the editor and keeps the find bar open; with an empty search field closes it.";
    /// Move between the search and the replace field.
    SwitchField => "find.switch-field", "Next Field",
        "Moves the focus between the search field and the replace field.";
    /// Break the line in a field.
    NewLine => "find.new-line", "New Line",
        "Breaks the line in the focused search or replace field (the find bar, Find in Files), for a search or a replacement over several lines.";
    /// Down in a field.
    Down => "find.down", "Next Occurrence or Line Down",
        "Selects the next match; while a field holds several lines, moves the caret down instead.";
    /// Up in a field.
    Up => "find.up", "Previous Occurrence or Line Up",
        "Selects the previous match; while a field holds several lines, moves the caret up instead.";
    /// Show the recent searches.
    ShowHistory => "find.show-history", "Search History",
        "Shows the recent searches (or replacements) of the session below the focused field.";
}

ui::ui_commands! {
    HISTORY_COMMANDS, scope = "FindHistory";
    HistoryUp => "find.history-up", "Previous Item", "Selects the item above in the search history.";
    HistoryDown => "find.history-down", "Next Item", "Selects the item below in the search history.";
    HistoryPick => "find.history-pick", "Use Item",
        "Puts the selected item of the search history into the field.";
    HistoryClose => "find.history-close", "Close History", "Closes the search history.";
}

/// Most matches one search collects; the counter shows `1000000+` beyond.
pub const MATCH_LIMIT: usize = 1_000_000;
/// Texts up to this size are searched on the UI thread (well under a
/// millisecond for a literal); larger ones on a background thread.
const SYNC_SEARCH_BYTES: usize = 256 * 1024;
/// Most entries of each search history.
pub const HISTORY_SIZE: usize = 30;
/// Height of a row of the bar and of its fields.
const ROW_HEIGHT: f32 = 34.0;
const FIELD_HEIGHT: f32 = 28.0;
/// Lines a search field shows before it scrolls (the reference IDE's
/// `ide.find.max.rows`).
pub const MAX_FIELD_ROWS: usize = 3;
/// Share of the bar's width taken by the fields, and their smallest width.
const FIELD_WIDTH: f32 = 0.4;
const MIN_FIELD_WIDTH: f32 = 240.0;
/// Room left of the fields (the mode button and the gaps).
const FIELD_LEFT: f32 = 4.0 + SMALL_BUTTON + 4.0;

/// The last search of the session, shared by the find bars of all editors:
/// a bar opens with it, and Find Next with a closed bar repeats it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastSearch {
    pub text: String,
    pub replace: String,
    pub options: SearchOptions,
    /// A search was made in this session; until then Find Next opens the
    /// find bar.
    pub performed: bool,
    /// Recent searches and replacements, newest first.
    pub history: Vec<String>,
    pub replace_history: Vec<String>,
}

impl Global for LastSearch {}

/// The last search of the session.
pub fn last_search(cx: &App) -> LastSearch {
    cx.try_global::<LastSearch>().cloned().unwrap_or_default()
}

fn update_last_search(cx: &mut App, f: impl FnOnce(&mut LastSearch)) {
    f(cx.default_global::<LastSearch>());
}

/// Puts a search first in the search history the find bars show; Find in
/// Files shares it, as in the reference IDE.
pub fn remember_search(cx: &mut App, text: &str) {
    update_last_search(cx, |s| remember(&mut s.history, text));
}

/// The replacement of the last search (Replace in Files shares it).
pub fn set_last_replace(cx: &mut App, text: &str) {
    update_last_search(cx, |s| s.replace = text.to_owned());
}

/// Puts a replacement first in the replace history.
pub fn remember_replace(cx: &mut App, text: &str) {
    update_last_search(cx, |s| remember(&mut s.replace_history, text));
}

/// Puts `text` first in a history.
fn remember(history: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    history.retain(|t| t != text);
    history.insert(0, text.to_owned());
    history.truncate(HISTORY_SIZE);
}

/// Which way Find Next and Find Previous go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// Where the last search stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Nothing to search for.
    Empty,
    /// A search is running; the previous matches stay on screen.
    Searching,
    Done,
    /// The regular expression does not compile.
    BadPattern,
    /// The regular expression matches only empty strings.
    Vague,
    /// In Selection is on but nothing was selected.
    NoSelection,
}

/// Why a search runs; decides what becomes the current match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// The query or an option changed, or the bar opened: the match at or
    /// after the caret becomes current and is selected.
    Query,
    /// The editor's text changed: the current match stays where it can and
    /// the selection is left alone.
    Edited,
    /// A match was replaced up to this char offset: the next match after
    /// it becomes current.
    Replaced(usize),
    /// The bar is closed: Find Next or Previous with the last search.
    Again(Direction),
}

/// Which field a history belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Query,
    Replace,
}

/// The open search history below a field.
struct HistoryPopup {
    field: Field,
    items: Vec<String>,
    selected: usize,
}

/// The query and options the shown matches belong to.
struct Applied {
    text: String,
    matcher: Matcher,
}

/// The find bar of one editor.
pub struct FindBar {
    editor: Entity<Editor>,
    query: Entity<Editor>,
    replace: Entity<Editor>,
    history_focus: FocusHandle,
    open: bool,
    replace_mode: bool,
    options: SearchOptions,
    /// Search only within this char range of the editor (In Selection).
    scope: Option<Range<usize>>,
    status: Status,
    truncated: bool,
    /// What the shown matches were found with.
    applied: Option<Applied>,
    /// The editor's text as last searched, with its buffer version.
    snapshot: Option<(u64, Arc<String>)>,
    search_task: Option<Task<()>>,
    cancel: Option<CancelToken>,
    replace_task: Option<Task<()>>,
    /// The next edit of the editor is our replacement ending here.
    replaced_to: Option<usize>,
    /// Next or Previous asked for while a search ran.
    pending: Option<Direction>,
    /// The last move found nothing this way; the next move this way starts
    /// over from the other end.
    wrap: Option<Direction>,
    /// What the last action said ("not found, press ... to search from the
    /// top"), shown near the caret until the next key.
    hint: Option<SharedString>,
    hint_above: bool,
    /// The editor's selections when the bar opened: restored when the query
    /// is cleared, and the range In Selection searches.
    initial: Option<SelectionSet>,
    /// The selection the bar gave the editor last (moving away from it
    /// ends a pending wrap and the hint).
    own_selection: Option<Range<usize>>,
    history: Option<HistoryPopup>,
    _subscriptions: Vec<Subscription>,
}

/// What a find bar tells its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindEvent {
    /// New matches are shown (a host may draw a frame at once instead of
    /// at the next display refresh).
    Updated,
}

impl gpui::EventEmitter<FindEvent> for FindBar {}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl FindBar {
    pub fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut e = Editor::new(Buffer::new(""), window, cx);
                e.set_single_line(true);
                e.set_ui_font(true);
                e.set_transparent(true);
                e.set_placeholder(Some(placeholder.into()));
                // A text field: no paired brackets or quotes.
                e.set_config(crate::EditorConfig {
                    brackets: Vec::new(),
                    ..crate::EditorConfig::default()
                });
                e
            })
        };
        let query = field("Search", window, cx);
        let replace = field("Replace", window, cx);
        let history_focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event, window, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    this.query_edited(window, cx);
                }
            }),
            cx.subscribe_in(&replace, window, |this, field, event, _, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    let text = field.read(cx).text();
                    update_last_search(cx, |s| s.replace = text);
                    this.replace.update(cx, |f, cx| f.set_error(false, cx));
                }
            }),
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                EditorEvent::Edited(applied) => this.editor_edited(applied, window, cx),
                EditorEvent::SelectionsChanged => this.editor_selection_changed(cx),
            }),
            cx.on_blur(&history_focus, window, |this, _, cx| {
                if this.history.take().is_some() {
                    cx.notify();
                }
            }),
        ];
        Self {
            editor,
            query,
            replace,
            history_focus,
            open: false,
            replace_mode: false,
            options: SearchOptions::default(),
            scope: None,
            status: Status::Empty,
            truncated: false,
            applied: None,
            snapshot: None,
            search_task: None,
            cancel: None,
            replace_task: None,
            replaced_to: None,
            pending: None,
            wrap: None,
            hint: None,
            hint_above: false,
            initial: None,
            own_selection: None,
            history: None,
            _subscriptions: subscriptions,
        }
    }

    // ---- state ------------------------------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_replace_mode(&self) -> bool {
        self.replace_mode
    }

    pub fn options(&self) -> SearchOptions {
        self.options
    }

    /// The editor the bar searches.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The search field.
    pub fn query_field(&self) -> &Entity<Editor> {
        &self.query
    }

    /// The replace field.
    pub fn replace_field(&self) -> &Entity<Editor> {
        &self.replace
    }

    /// The query the shown matches were found with (measurements).
    pub fn searched_text(&self) -> Option<&str> {
        self.applied.as_ref().map(|a| a.text.as_str())
    }

    /// What the last action said, shown near the caret until the next key.
    pub fn hint(&self) -> Option<&SharedString> {
        self.hint.as_ref()
    }

    /// The hint goes above the caret (a backward search), not below it.
    pub fn hint_above(&self) -> bool {
        self.hint_above
    }

    pub fn clear_hint(&mut self, cx: &mut Context<Self>) {
        if self.hint.take().is_some() {
            cx.notify();
        }
    }

    /// A search or a Replace All is running.
    pub fn is_searching(&self) -> bool {
        self.search_task.is_some() || self.replace_task.is_some()
    }

    /// Number of matches and the one-based index of the current match.
    pub fn counts(&self, cx: &App) -> (usize, Option<usize>) {
        self.editor
            .read(cx)
            .search_highlights()
            .map_or((0, None), |h| (h.matches.len(), h.current.map(|i| i + 1)))
    }

    fn current_excluded(&self, cx: &App) -> bool {
        self.editor
            .read(cx)
            .search_highlights()
            .and_then(|h| h.current.map(|i| h.excluded.contains(&i)))
            .unwrap_or(false)
    }

    /// The counter text and whether it reports a problem (nothing found, a
    /// bad pattern).
    pub fn status(&self, cx: &App) -> (String, bool) {
        let (count, current) = self.counts(cx);
        match self.status {
            Status::Empty => (results(0), false),
            Status::BadPattern => ("Bad pattern".into(), true),
            Status::Vague => ("Vague pattern".into(), false),
            Status::NoSelection => ("No selection".into(), false),
            Status::Searching | Status::Done => {
                if self.truncated {
                    (format!("{count}+"), false)
                } else if let Some(current) = current {
                    (format!("{current}/{count}"), false)
                } else {
                    (results(count), count == 0 && self.status == Status::Done)
                }
            }
        }
    }

    /// The bar's state as JSON (`find.state`).
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let (count, current) = self.counts(cx);
        let (status, error) = self.status(cx);
        let editor = self.editor.read(cx);
        let point = |p: usize| {
            let p = editor.buffer().offset_to_point(p);
            json!({ "line": p.line, "column": p.column })
        };
        let highlights = editor.search_highlights();
        let current_range = highlights
            .and_then(|h| h.current_range())
            .map(|r| json!({ "start": point(r.start), "end": point(r.end) }));
        json!({
            "open": self.open,
            "replace_mode": self.replace_mode,
            "text": self.query.read(cx).text(),
            "replace": self.replace.read(cx).text(),
            "case_sensitive": self.options.case_sensitive,
            "whole_words": self.options.whole_words,
            "regex": self.options.regex,
            "in_selection": self.scope.is_some(),
            "matches": count,
            "current": current,
            "current_range": current_range,
            "excluded": highlights.map_or(0, |h| h.excluded.len()),
            "truncated": self.truncated,
            "searching": self.is_searching(),
            "status": status,
            "error": error,
            "hint": self.hint.as_ref().map(|h| h.to_string()),
            "history": self.history.as_ref().map(|h| json!({
                "items": h.items,
                "selected": h.selected,
            })),
        })
    }

    // ---- opening and closing ---------------------------------------------

    /// Opens the bar (or brings the focus back to it), as the reference IDE
    /// does: the selected text becomes the query, also over several lines;
    /// only Replace with a selection over several lines starts with an
    /// empty query and searches within the selection. Otherwise the field
    /// keeps the last search (unless that was over several lines). The
    /// field texts are selected so that typing replaces them.
    pub fn open(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let last = last_search(cx);
        let was_open = self.open;
        if !was_open {
            self.options = last.options;
        }
        self.replace_mode = replace;
        self.history = None;
        let (selection, selected, current) = {
            let e = self.editor.read(cx);
            let range = e.selections().primary().range();
            let current = e.search_highlights().and_then(|h| h.current_range());
            (range.clone(), e.buffer().slice(range), current)
        };
        let mut text = if was_open {
            self.query.read(cx).text()
        } else {
            last.text.clone()
        };
        if !was_open && text.contains('\n') {
            text.clear();
        }
        // The selected current match of an open bar is not a new query.
        let selection_is_match = was_open && current.as_ref() == Some(&selection);
        if !selection_is_match {
            self.initial = Some(self.editor.read(cx).selections().clone());
            self.scope = None;
            if !selected.is_empty() {
                if replace && selected.contains('\n') {
                    // Replace within the lines, starting with an empty query.
                    self.scope = Some(selection);
                    text.clear();
                } else {
                    text = if self.options.regex {
                        search::escape(&selected)
                    } else {
                        selected
                    };
                    let options = self.options;
                    update_last_search(cx, |s| {
                        s.text = text.clone();
                        s.options = options;
                        s.performed = true;
                    });
                }
            }
        }
        self.open = true;
        self.wrap = None;
        self.hint = None;
        self.pending = None;
        self.update_placeholder(cx);
        self.query
            .update(cx, |f, cx| f.set_text_selected(&text, cx));
        let replace_text = if was_open {
            self.replace.read(cx).text()
        } else {
            last.replace
        };
        self.replace
            .update(cx, |f, cx| f.set_text_selected(&replace_text, cx));
        window.focus(&self.query.focus_handle(cx));
        self.search(Reason::Query, window, cx);
        cx.notify();
    }

    /// Closes the bar: the matches are no longer shown, the current match
    /// stays selected and the editor gets the focus.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.cancel_search();
        self.replace_task = None;
        self.pending = None;
        self.hint = None;
        self.history = None;
        self.snapshot = None;
        self.applied = None;
        self.editor
            .update(cx, |e, cx| e.set_search_highlights(None, cx));
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Opens the bar and searches for `text` with the given options.
    pub fn search_for(
        &mut self,
        text: &str,
        options: SearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            self.open(self.replace_mode, window, cx);
        }
        self.set_options(options, cx);
        self.query.update(cx, |f, cx| f.set_text_selected(text, cx));
        self.query_edited(window, cx);
    }

    fn update_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = if self.scope.is_some() {
            "Search in selection only"
        } else {
            "Search"
        };
        self.query
            .update(cx, |f, _| f.set_placeholder(Some(placeholder.into())));
    }

    // ---- searching ------------------------------------------------------

    fn set_options(&mut self, options: SearchOptions, cx: &mut Context<Self>) {
        self.options = options;
        update_last_search(cx, |s| s.options = options);
    }

    fn query_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.read(cx).text();
        update_last_search(cx, |s| {
            s.text = text;
            s.performed = true;
        });
        self.wrap = None;
        self.hint = None;
        self.search(Reason::Query, window, cx);
    }

    fn toggle_option(
        &mut self,
        f: impl FnOnce(&mut SearchOptions),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut options = self.options;
        f(&mut options);
        self.set_options(options, cx);
        self.wrap = None;
        self.hint = None;
        if self.open {
            self.search(Reason::Query, window, cx);
        }
        cx.notify();
    }

    fn cancel_search(&mut self) {
        self.search_task = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    /// The editor's text changed: the In Selection range moves with it, and
    /// the bar searches again.
    fn editor_edited(
        &mut self,
        applied: &[AppliedEdit],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(scope) = &mut self.scope {
            *scope = map_range(scope.clone(), applied);
        }
        if !self.open {
            self.replaced_to = None;
            return;
        }
        let reason = match self.replaced_to.take() {
            Some(end) => {
                if let Some(scope) = self.scope.clone() {
                    // The selection keeps showing the searched range.
                    self.own_selection = Some(scope.clone());
                    self.editor.update(cx, |e, cx| {
                        e.set_selections(
                            SelectionSet::single(Selection::new(scope.start, scope.end)),
                            cx,
                        )
                    });
                }
                Reason::Replaced(end)
            }
            None => Reason::Edited,
        };
        self.search(reason, window, cx);
    }

    /// Starts a search on a snapshot of the editor's text; the result is
    /// applied when it comes back, unless a newer search replaced it.
    fn search(&mut self, reason: Reason, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_search();
        let again = matches!(reason, Reason::Again(_));
        let query = if again {
            let last = last_search(cx);
            Query::new(last.text, last.options)
        } else {
            Query::new(self.query.read(cx).text(), self.options)
        };
        let scope = if again { None } else { self.scope.clone() };
        let compiled = match query.compile() {
            Ok(_) if scope.as_ref().is_some_and(|s| s.is_empty()) => Err(Status::NoSelection),
            Ok(matcher) => Ok(matcher),
            Err(QueryError::Empty) => Err(Status::Empty),
            Err(QueryError::Vague) => Err(Status::Vague),
            Err(QueryError::InvalidRegex(_)) => Err(Status::BadPattern),
        };
        let matcher = match compiled {
            Ok(matcher) => matcher,
            Err(status) => {
                if !again {
                    self.status = status;
                    self.truncated = false;
                    self.applied = None;
                    let restore = (status == Status::Empty && reason == Reason::Query)
                        .then(|| self.initial.clone())
                        .flatten();
                    self.editor.update(cx, |ed, cx| {
                        ed.set_search_highlights(None, cx);
                        if let Some(selections) = restore {
                            ed.set_selections(selections, cx);
                        }
                    });
                    self.update_field_error(cx);
                    cx.notify();
                }
                return;
            }
        };
        let (rope, version) = {
            let e = self.editor.read(cx);
            (e.buffer().rope().clone(), e.buffer().version())
        };
        let cached = self
            .snapshot
            .as_ref()
            .filter(|(v, _)| *v == version)
            .map(|(_, text)| text.clone());
        let byte_scope = scope.map(|r| {
            let end = r.end.min(rope.len_chars());
            rope.char_to_byte(r.start.min(end))..rope.char_to_byte(end)
        });
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        if !again {
            self.status = Status::Searching;
        }
        let text = query.text.clone();
        if rope.len_bytes() <= SYNC_SEARCH_BYTES {
            // A small text is searched at once, so that the frame drawn for
            // the key already shows the matches.
            self.cancel = None;
            if let Ok((snapshot, ranges, lines, truncated)) =
                run_search(cached, rope, &matcher, byte_scope, &cancel)
            {
                let found = Found {
                    reason,
                    version,
                    applied: Applied { text, matcher },
                    ranges,
                    lines,
                    truncated,
                };
                self.search_done(found, snapshot, window, cx);
            }
            return;
        }
        let work_matcher = matcher.clone();
        let work = cx.background_spawn(async move {
            run_search(cached, rope, &work_matcher, byte_scope, &cancel)
        });
        self.search_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.search_task = None;
                this.cancel = None;
                if let Ok((snapshot, ranges, lines, truncated)) = result {
                    let found = Found {
                        reason,
                        version,
                        applied: Applied { text, matcher },
                        ranges,
                        lines,
                        truncated,
                    };
                    this.search_done(found, snapshot, window, cx);
                }
            });
        }));
        cx.notify();
    }

    fn search_done(
        &mut self,
        found: Found,
        snapshot: Arc<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reason = found.reason;
        if self.editor.read(cx).buffer().version() != found.version {
            // The text changed while the search ran.
            self.search(reason, window, cx);
            return;
        }
        self.snapshot = Some((found.version, snapshot));
        if let Reason::Again(direction) = reason {
            self.move_without_bar(direction, &found.ranges, window, cx);
            return;
        }
        if !self.open {
            return;
        }
        self.status = Status::Done;
        self.truncated = found.truncated;
        self.applied = Some(found.applied);
        let matches = found.ranges;
        let (old, old_excluded, caret, selection) = {
            let e = self.editor.read(cx);
            let s = e.selections().primary();
            let h = e.search_highlights();
            let excluded: Vec<Range<usize>> = h
                .map(|h| {
                    h.excluded
                        .iter()
                        .filter_map(|&i| h.matches.get(i).cloned())
                        .collect()
                })
                .unwrap_or_default();
            (
                h.and_then(|h| h.current_range()),
                excluded,
                s.head,
                s.range(),
            )
        };
        let current = match reason {
            Reason::Replaced(end) => {
                let i = matches.partition_point(|m| m.start < end);
                if i < matches.len() {
                    Some(i)
                } else {
                    self.not_found(Direction::Forward, !matches.is_empty(), window, cx);
                    None
                }
            }
            _ => pick_current(&matches, old, caret, selection),
        };
        // Exclusions stay with the matches they were made on.
        let excluded = old_excluded
            .iter()
            .filter_map(|r| matches.binary_search_by(|m| m.start.cmp(&r.start)).ok())
            .filter(|&i| old_excluded.contains(&matches[i]))
            .collect();
        let target = current.map(|i| matches[i].clone());
        if let Some(target) = &target {
            self.own_selection = Some(target.clone());
        }
        let in_selection = self.scope.is_some();
        let select = matches!(reason, Reason::Query | Reason::Replaced(_));
        let highlights = SearchHighlights {
            matches,
            current,
            current_background: in_selection,
            excluded,
            lines: found.lines,
        };
        self.editor.update(cx, |e, cx| {
            e.set_search_highlights(Some(highlights), cx);
            if let (true, Some(range)) = (select, target) {
                if in_selection {
                    e.reveal(range.start, cx);
                } else {
                    e.select_and_reveal(range, cx);
                }
            }
        });
        self.update_field_error(cx);
        if let Some(direction) = self.pending.take() {
            self.step(direction, window, cx);
        }
        cx.emit(FindEvent::Updated);
        cx.notify();
    }

    fn update_field_error(&mut self, cx: &mut Context<Self>) {
        let (_, error) = self.status(cx);
        let error = error && self.status != Status::Searching;
        self.query.update(cx, |f, cx| f.set_error(error, cx));
    }

    // ---- moving between matches -------------------------------------------

    /// Find Next or Previous: with the bar open the next match of its
    /// search; with the bar closed the next match of the last search (or
    /// the bar opens when there was none).
    pub fn find(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            let text = self.query.read(cx).text();
            update_last_search(cx, |s| remember(&mut s.history, &text));
            self.step(direction, window, cx);
            return;
        }
        let last = last_search(cx);
        if !last.performed || last.text.is_empty() {
            self.open(false, window, cx);
            return;
        }
        self.search(Reason::Again(direction), window, cx);
    }

    /// Moves to the next or previous match of the open bar.
    fn step(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_task.is_some() {
            self.pending = Some(direction);
            return;
        }
        let in_selection = self.scope.is_some();
        let wrap = self.wrap == Some(direction);
        let target = {
            let e = self.editor.read(cx);
            let Some(h) = e.search_highlights().filter(|h| !h.matches.is_empty()) else {
                return;
            };
            let len = h.matches.len();
            if wrap {
                Some(match direction {
                    Direction::Forward => 0,
                    Direction::Backward => len - 1,
                })
            } else {
                let caret = e.selections().primary().head;
                let from = match (in_selection, h.current_range()) {
                    (true, Some(current)) => current,
                    _ => caret..caret,
                };
                neighbour(&h.matches, h.current, from, direction)
            }
        };
        match target {
            Some(i) => {
                self.wrap = None;
                self.hint = None;
                self.go_to_match(i, cx);
            }
            None => self.not_found(direction, true, window, cx),
        }
        cx.notify();
    }

    /// Makes match `i` current: selected (or, within a selection, scrolled
    /// to).
    fn go_to_match(&mut self, i: usize, cx: &mut Context<Self>) {
        let in_selection = self.scope.is_some();
        self.own_selection = self
            .editor
            .read(cx)
            .search_highlights()
            .and_then(|h| h.matches.get(i).cloned());
        self.editor.update(cx, |e, cx| {
            e.set_search_current(Some(i), cx);
            let range = e
                .search_highlights()
                .and_then(|h| h.matches.get(i).cloned());
            if let Some(range) = range {
                if in_selection {
                    e.reveal(range.start, cx);
                } else {
                    e.select_and_reveal(range, cx);
                }
            }
        });
    }

    /// Find Next or Previous with the bar closed, on the matches of the
    /// last search.
    fn move_without_bar(
        &mut self,
        direction: Direction,
        matches: &[Range<usize>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.editor.read(cx).selections().primary().range();
        let target = if matches.is_empty() {
            None
        } else if self.wrap == Some(direction) {
            Some(match direction {
                Direction::Forward => 0,
                Direction::Backward => matches.len() - 1,
            })
        } else {
            match direction {
                Direction::Forward => {
                    let i = matches.partition_point(|m| m.start < selection.end);
                    let i = if matches.get(i) == Some(&selection) {
                        i + 1
                    } else {
                        i
                    };
                    (i < matches.len()).then_some(i)
                }
                Direction::Backward => matches
                    .partition_point(|m| m.end < selection.start.max(1))
                    .checked_sub(1),
            }
        };
        match target {
            Some(i) => {
                self.wrap = None;
                self.hint = None;
                let range = matches[i].clone();
                self.own_selection = Some(range.clone());
                self.editor
                    .update(cx, |e, cx| e.select_and_reveal(range, cx));
            }
            None => self.not_found(direction, !matches.is_empty(), window, cx),
        }
        cx.notify();
    }

    /// Nothing more this way: say so; the next move this way wraps around
    /// when there is something to wrap to.
    fn not_found(
        &mut self,
        direction: Direction,
        can_wrap: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = if self.open {
            self.query.read(cx).text()
        } else {
            last_search(cx).text
        };
        let mut message = format!("\"{}\" not found", one_line(&text));
        if can_wrap {
            self.wrap = Some(direction);
            let editor_focus = self.editor.focus_handle(cx);
            let (shortcut, place, action) = match direction {
                Direction::Forward => (
                    ui::keys::shortcut_in(&Next, &editor_focus, window),
                    "top",
                    "Find Next",
                ),
                Direction::Backward => (
                    ui::keys::shortcut_in(&Previous, &editor_focus, window),
                    "bottom",
                    "Find Previous",
                ),
            };
            message = match shortcut {
                Some(keys) => format!("{message}, press {keys} to search from the {place}"),
                None => format!("{message}, perform \"{action}\" again to search from the {place}"),
            };
        }
        self.hint = Some(message.into());
        self.hint_above = direction == Direction::Backward;
    }

    /// The caret moved: onto another match, that match becomes current (the
    /// selection stays); elsewhere a move that went nowhere no longer wraps.
    fn editor_selection_changed(&mut self, cx: &mut Context<Self>) {
        if !self.open && self.wrap.is_none() && self.hint.is_none() {
            return;
        }
        let (selection, current, at_caret) = {
            let e = self.editor.read(cx);
            let selection = e.selections().primary().range();
            let caret = e.selections().primary().head;
            let h = e.search_highlights();
            let at_caret = h.and_then(|h| {
                let i = h.first_ending_at_or_after(caret);
                h.matches.get(i).filter(|m| m.start <= caret).map(|_| i)
            });
            (selection, h.and_then(|h| h.current_range()), at_caret)
        };
        let ours =
            current.as_ref() == Some(&selection) || self.own_selection.as_ref() == Some(&selection);
        if !ours {
            if self.wrap.take().is_some() || self.hint.take().is_some() {
                cx.notify();
            }
            if self.open
                && self.search_task.is_none()
                && let Some(i) = at_caret
            {
                self.editor
                    .update(cx, |e, cx| e.set_search_current(Some(i), cx));
            }
        }
    }

    // ---- replacing ----------------------------------------------------------

    /// The shown matches belong to the current text.
    fn results_current(&self, cx: &App) -> bool {
        let version = self.editor.read(cx).buffer().version();
        self.search_task.is_none()
            && self.applied.is_some()
            && self.snapshot.as_ref().is_some_and(|(v, _)| *v == version)
    }

    /// A read-only file is not changed; the hint says so.
    fn writable(&mut self, cx: &mut Context<Self>) -> bool {
        if self.editor.read(cx).buffer().is_read_only() {
            self.hint = Some("The file is read-only".into());
            self.hint_above = false;
            cx.notify();
            return false;
        }
        true
    }

    fn remember_both(&self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).text();
        let replace = self.replace.read(cx).text();
        update_last_search(cx, |s| {
            remember(&mut s.history, &text);
            remember(&mut s.replace_history, &replace);
        });
    }

    fn replace_failed(&mut self, error: ReplaceError, window: &mut Window, cx: &mut Context<Self>) {
        match error {
            ReplaceError::Malformed(message) => {
                self.hint = Some(format!("Malformed replacement string: {message}").into());
                self.hint_above = false;
                self.replace.update(cx, |f, cx| f.set_error(true, cx));
            }
            ReplaceError::Stale => self.search(Reason::Edited, window, cx),
        }
        cx.notify();
    }

    /// Replaces the current match; the next match becomes current.
    pub fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || !self.replace_mode || !self.results_current(cx) || !self.writable(cx) {
            return;
        }
        let Some(range) = self
            .editor
            .read(cx)
            .search_highlights()
            .and_then(|h| h.current_range())
        else {
            return;
        };
        let template = self.replace.read(cx).text();
        let (Some(applied), Some((_, text))) = (&self.applied, &self.snapshot) else {
            return;
        };
        let rope = self.editor.read(cx).buffer().rope().clone();
        let bytes = rope.char_to_byte(range.start)..rope.char_to_byte(range.end);
        match applied.matcher.replacement(text, bytes, &template) {
            Ok(new) => {
                self.remember_both(cx);
                let end = range.start + new.chars().count();
                self.replaced_to = Some(end);
                self.own_selection = Some(end..end);
                self.hint = None;
                self.wrap = None;
                self.editor.update(cx, |e, cx| {
                    e.edit(
                        vec![Edit::new(range, new)],
                        EditKind::Other,
                        CaretPlacement::Explicit(SelectionSet::caret(end)),
                        cx,
                    )
                });
            }
            Err(e) => self.replace_failed(e, window, cx),
        }
    }

    /// Replaces every match that is not excluded, as one undo step (the
    /// replacements of a regular expression are built off the UI thread).
    pub fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || !self.replace_mode || !self.results_current(cx) || !self.writable(cx) {
            return;
        }
        let template = self.replace.read(cx).text();
        let (Some(applied), Some((version, text))) = (&self.applied, &self.snapshot) else {
            return;
        };
        let (version, text, matcher) = (*version, text.clone(), applied.matcher.clone());
        let ranges: Vec<Range<usize>> = {
            let e = self.editor.read(cx);
            let Some(h) = e.search_highlights() else {
                return;
            };
            h.matches
                .iter()
                .enumerate()
                .filter(|(i, _)| !h.excluded.contains(i))
                .map(|(_, r)| r.clone())
                .collect()
        };
        if ranges.is_empty() {
            return;
        }
        self.remember_both(cx);
        let work = cx.background_spawn(async move {
            let mut bytes = ranges.clone();
            to_byte_ranges(&text, &mut bytes);
            let mut edits = Vec::with_capacity(ranges.len());
            for (chars, bytes) in ranges.into_iter().zip(bytes) {
                let new = matcher.replacement(&text, bytes, &template)?;
                edits.push(Edit::new(chars, new));
            }
            Ok::<_, ReplaceError>(edits)
        });
        self.replace_task = Some(cx.spawn_in(window, async move |this, cx| {
            let edits = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.replace_task = None;
                match edits {
                    Ok(edits) => this.apply_replace_all(version, edits, window, cx),
                    Err(e) => this.replace_failed(e, window, cx),
                }
            });
        }));
        cx.notify();
    }

    fn apply_replace_all(
        &mut self,
        version: u64,
        edits: Vec<Edit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.read(cx).buffer().version() != version {
            // The text changed meanwhile; the user can try again on the new
            // matches.
            return;
        }
        let count = edits.len();
        // The caret goes after the last replacement.
        let mut delta: isize = 0;
        let mut caret = 0;
        for e in &edits {
            let len = e.text.chars().count() as isize;
            caret = (e.range.start as isize + delta + len) as usize;
            delta += len - e.range.len() as isize;
        }
        let selection = match self.scope.clone() {
            Some(scope) => {
                Selection::new(scope.start, (scope.end as isize + delta).max(0) as usize)
            }
            None => Selection::caret(caret),
        };
        self.wrap = None;
        self.own_selection = Some(selection.range());
        self.hint_above = false;
        self.hint = Some(
            match count {
                1 => "1 occurrence replaced".to_owned(),
                n => format!("{n} occurrences replaced"),
            }
            .into(),
        );
        self.editor.update(cx, |e, cx| {
            e.edit(
                edits,
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::single(selection)),
                cx,
            )
        });
        let _ = window;
        cx.notify();
    }

    /// Leaves the current match out of Replace All (or takes it back) and
    /// goes to the next one.
    pub fn exclude(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || !self.replace_mode {
            return;
        }
        let Some((i, excluded)) = self
            .editor
            .read(cx)
            .search_highlights()
            .and_then(|h| h.current.map(|i| (i, h.excluded.contains(&i))))
        else {
            return;
        };
        self.editor
            .update(cx, |e, cx| e.set_search_excluded(i, !excluded, cx));
        self.step(Direction::Forward, window, cx);
    }

    // ---- other actions -------------------------------------------------------

    fn toggle_in_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        if self.scope.take().is_none() {
            // The selection the bar opened with (now the current match may
            // be selected).
            let initial = self.initial.clone().unwrap_or_default();
            self.scope = Some(initial.primary().range());
            self.editor
                .update(cx, |e, cx| e.set_selections(initial, cx));
        }
        self.update_placeholder(cx);
        self.wrap = None;
        self.hint = None;
        self.search(Reason::Query, window, cx);
        cx.notify();
    }

    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.read(cx).text();
        if text.is_empty() {
            self.close(window, cx);
            return;
        }
        update_last_search(cx, |s| remember(&mut s.history, &text));
        window.focus(&self.editor.focus_handle(cx));
    }

    fn switch_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replace_mode {
            return;
        }
        let target = if self.query.focus_handle(cx).is_focused(window) {
            &self.replace
        } else {
            &self.query
        };
        let handle = target.focus_handle(cx);
        target.update(cx, |f, cx| {
            let len = f.buffer().len_chars();
            f.set_selections(SelectionSet::single(Selection::new(0, len)), cx);
        });
        window.focus(&handle);
    }

    fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = if self.replace.focus_handle(cx).is_focused(window) {
            Field::Replace
        } else {
            Field::Query
        };
        let last = last_search(cx);
        let items = match field {
            Field::Query => last.history,
            Field::Replace => last.replace_history,
        };
        if items.is_empty() {
            return;
        }
        self.history = Some(HistoryPopup {
            field,
            items,
            selected: 0,
        });
        window.focus(&self.history_focus);
        cx.notify();
    }

    fn history_move(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.history {
            let len = h.items.len();
            h.selected = if down {
                (h.selected + 1) % len
            } else {
                (h.selected + len - 1) % len
            };
            cx.notify();
        }
    }

    fn history_pick(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = self.history.take() else {
            return;
        };
        let text = h.items[index.unwrap_or(h.selected)].clone();
        let field = match h.field {
            Field::Query => self.query.clone(),
            Field::Replace => self.replace.clone(),
        };
        field.update(cx, |f, cx| f.set_text_selected(&text, cx));
        window.focus(&field.focus_handle(cx));
        match h.field {
            Field::Query => self.query_edited(window, cx),
            Field::Replace => update_last_search(cx, |s| s.replace = text),
        }
        cx.notify();
    }

    fn history_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.history.take() {
            let field = match h.field {
                Field::Query => &self.query,
                Field::Replace => &self.replace,
            };
            window.focus(&field.focus_handle(cx));
            cx.notify();
        }
    }

    // ---- actions ------------------------------------------------------------

    fn open_action(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        self.open(false, window, cx);
    }

    fn open_replace_action(
        &mut self,
        _: &OpenReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(true, window, cx);
    }

    fn next_action(&mut self, _: &Next, window: &mut Window, cx: &mut Context<Self>) {
        self.find(Direction::Forward, window, cx);
    }

    fn previous_action(&mut self, _: &Previous, window: &mut Window, cx: &mut Context<Self>) {
        self.find(Direction::Backward, window, cx);
    }

    fn search_action(&mut self, a: &Search, window: &mut Window, cx: &mut Context<Self>) {
        let mut options = if self.open {
            self.options
        } else {
            last_search(cx).options
        };
        options.case_sensitive = a.case_sensitive.unwrap_or(options.case_sensitive);
        options.whole_words = a.whole_words.unwrap_or(options.whole_words);
        options.regex = a.regex.unwrap_or(options.regex);
        if let Some(replace) = &a.replace {
            self.replace_mode = true;
            self.replace
                .update(cx, |f, cx| f.set_text_selected(replace, cx));
            let replace = replace.clone();
            update_last_search(cx, |s| s.replace = replace);
        }
        self.search_for(&a.text, options, window, cx);
    }

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn toggle_case_action(&mut self, _: &ToggleCase, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.case_sensitive = !o.case_sensitive, window, cx);
    }

    fn toggle_words_action(
        &mut self,
        _: &ToggleWords,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_option(|o| o.whole_words = !o.whole_words, window, cx);
    }

    fn toggle_regex_action(
        &mut self,
        _: &ToggleRegex,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_option(|o| o.regex = !o.regex, window, cx);
    }

    fn toggle_in_selection_action(
        &mut self,
        _: &ToggleInSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_in_selection(window, cx);
    }

    fn replace_current_action(
        &mut self,
        _: &ReplaceCurrent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_current(window, cx);
    }

    fn replace_all_action(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_all(window, cx);
    }

    fn exclude_action(&mut self, _: &Exclude, window: &mut Window, cx: &mut Context<Self>) {
        self.exclude(window, cx);
    }

    fn focus_editor_action(
        &mut self,
        _: &FocusEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_editor(window, cx);
    }

    fn switch_field_action(
        &mut self,
        _: &SwitchField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_field(window, cx);
    }

    fn show_history_action(
        &mut self,
        _: &ShowHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_history(window, cx);
    }

    fn history_up_action(&mut self, _: &HistoryUp, _: &mut Window, cx: &mut Context<Self>) {
        self.history_move(false, cx);
    }

    fn history_down_action(&mut self, _: &HistoryDown, _: &mut Window, cx: &mut Context<Self>) {
        self.history_move(true, cx);
    }

    fn history_pick_action(
        &mut self,
        _: &HistoryPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_pick(None, window, cx);
    }

    fn history_close_action(
        &mut self,
        _: &HistoryClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_close(window, cx);
    }

    /// A field holds several lines: the search runs over line breaks, and
    /// Up and Down move the caret in the fields (as in the reference IDE).
    pub fn is_multiline(&self, cx: &App) -> bool {
        self.query.read(cx).buffer().len_lines() > 1
            || self.replace.read(cx).buffer().len_lines() > 1
    }

    /// The field that has the focus (the search field when none has).
    fn focused_field(&self, window: &Window, cx: &App) -> Entity<Editor> {
        if self.replace.focus_handle(cx).is_focused(window) {
            self.replace.clone()
        } else {
            self.query.clone()
        }
    }

    fn new_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.focused_field(window, cx);
        window.focus(&field.focus_handle(cx));
        field.update(cx, |f, cx| f.insert("\n", cx));
        cx.notify();
    }

    /// Up and Down in a field: the previous or next match, or, while a
    /// field holds several lines, the caret movement (the key goes on to the
    /// field's own binding).
    fn up_down(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_multiline(cx) {
            cx.propagate();
            return;
        }
        self.find(direction, window, cx);
    }

    fn new_line_action(&mut self, _: &NewLine, window: &mut Window, cx: &mut Context<Self>) {
        self.new_line(window, cx);
    }

    fn down_action(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.up_down(Direction::Forward, window, cx);
    }

    fn up_action(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.up_down(Direction::Backward, window, cx);
    }

    // ---- drawing --------------------------------------------------------------

    /// Lines a field shows: its lines, at most [`MAX_FIELD_ROWS`].
    fn field_rows(field: &Entity<Editor>, cx: &App) -> usize {
        field.read(cx).buffer().len_lines().clamp(1, MAX_FIELD_ROWS)
    }

    /// Height of a field's frame for its lines.
    fn frame_height(field: &Entity<Editor>, window: &Window, cx: &App) -> gpui::Pixels {
        let line = crate::field_line_height(window, cx);
        px(FIELD_HEIGHT) + line * (Self::field_rows(field, cx) - 1) as f32
    }

    /// Height of a row of the bar with this field.
    fn row_height(field: &Entity<Editor>, window: &Window, cx: &App) -> gpui::Pixels {
        Self::frame_height(field, window, cx) + px(ROW_HEIGHT - FIELD_HEIGHT)
    }

    /// The new-line button inside a field.
    fn new_line_button(
        &self,
        id: &'static str,
        field: &Entity<Editor>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let shortcut = ui::keys::shortcut_in(&NewLine, &field.focus_handle(cx), window)
            .map(SharedString::from);
        let target = field.clone();
        small_icon_button(id, IconName::NEW_LINE, true, cx)
            .tooltip(Tooltip::text("New Line", shortcut))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&target.focus_handle(cx));
                target.update(cx, |f, cx| f.insert("\n", cx));
            }))
    }

    /// A field of the bar: the frame, an optional leading icon, the one-line
    /// editor and trailing children.
    fn field_frame(
        &self,
        id: &'static str,
        field: &Entity<Editor>,
        leading: Option<IconName>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let focused = field.focus_handle(cx).is_focused(window)
            || (self
                .history
                .as_ref()
                .is_some_and(|h| (h.field == Field::Query) == (id == "find-query"))
                && self.history_focus.is_focused(window));
        let line = crate::field_line_height(window, cx);
        let rows = Self::field_rows(field, cx);
        // Inside the 1 px border: the first line centred in a one-line
        // field; more lines grow the field downwards, up to the most it
        // shows, then the text scrolls. Icons and buttons stay at the top.
        let inner = px(FIELD_HEIGHT - 2.0);
        let pad = ((inner - line) / 2.0).max(px(0.0));
        input_frame(focused, false, cx)
            .id(id)
            .debug_selector(move || id.into())
            .items_start()
            .w(relative(FIELD_WIDTH))
            .min_w(px(MIN_FIELD_WIDTH))
            .h(Self::frame_height(field, window, cx))
            .map(|d| match leading {
                Some(icon) => d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(inner)
                        .pl(px(6.0))
                        .pr(px(4.0))
                        .child(Icon::new(icon)),
                ),
                None => d.pl(px(8.0)),
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .mt(pad)
                    .h(line * rows as f32)
                    .overflow_hidden()
                    .child(field.clone()),
            )
    }

    /// Icons and buttons inside a field, at the height of its first line.
    fn field_trailing(&self) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(FIELD_HEIGHT - 2.0))
            .gap(px(2.0))
            .pr(px(3.0))
    }

    /// An element next to a field, at the height of the field's first line.
    fn beside_field(el: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(FIELD_HEIGHT))
            .child(el)
    }

    fn render_query_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, error) = self.status(cx);
        let (count, _) = self.counts(cx);
        let editor_focus = self.editor.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &editor_focus, window).map(SharedString::from)
        };
        let field_focus = self.query.focus_handle(cx);
        let field_shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &field_focus, window).map(SharedString::from)
        };
        let toggles = self
            .field_trailing()
            .child(self.new_line_button("find-query-new-line", &self.query, window, cx))
            .child(
                toggle_button(
                    "find-match-case",
                    IconName::MATCH_CASE,
                    self.options.case_sensitive,
                    cx,
                )
                .tooltip(Tooltip::text("Match Case", field_shortcut(&ToggleCase)))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_case_action(&ToggleCase, window, cx)
                })),
            )
            .child(
                toggle_button("find-words", IconName::WORDS, self.options.whole_words, cx)
                    .tooltip(Tooltip::text("Words", field_shortcut(&ToggleWords)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_words_action(&ToggleWords, window, cx)
                    })),
            )
            .child(
                toggle_button("find-regex", IconName::REGEX, self.options.regex, cx)
                    .tooltip(Tooltip::text("Regex", field_shortcut(&ToggleRegex)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_regex_action(&ToggleRegex, window, cx)
                    })),
            );
        let field = self
            .field_frame(
                "find-query",
                &self.query,
                Some(IconName::SEARCH_HISTORY),
                window,
                cx,
            )
            .child(toggles);
        let has_matches = count > 0;
        let previous = small_icon_button("find-previous", IconName::ARROW_UP, has_matches, cx)
            .tooltip(Tooltip::text("Previous Occurrence", shortcut(&Previous)))
            .when(has_matches, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.find(Direction::Backward, window, cx)
                }))
            });
        let next = small_icon_button("find-next", IconName::ARROW_DOWN, has_matches, cx)
            .tooltip(Tooltip::text("Next Occurrence", shortcut(&Next)))
            .when(has_matches, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.find(Direction::Forward, window, cx)
                }))
            });
        let in_selection = toggle_button(
            "find-in-selection",
            IconName::FILTER,
            self.scope.is_some(),
            cx,
        )
        .tooltip(Tooltip::text(
            "In Selection",
            field_shortcut(&ToggleInSelection),
        ))
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_in_selection(window, cx)),
        );
        let (mode_icon, mode_tooltip, mode_action): (_, _, Box<dyn gpui::Action>) =
            if self.replace_mode {
                (IconName::CHEVRON_DOWN, "Switch to Find", Box::new(Open))
            } else {
                (
                    IconName::CHEVRON_RIGHT,
                    "Switch to Replace",
                    Box::new(OpenReplace),
                )
            };
        let mode_shortcut = shortcut(mode_action.as_ref());
        div()
            .flex()
            .flex_row()
            .items_start()
            .h(Self::row_height(&self.query, window, cx))
            .px(px(4.0))
            .py(px((ROW_HEIGHT - FIELD_HEIGHT) / 2.0))
            .gap(px(4.0))
            .child(Self::beside_field(
                small_icon_button("find-mode", mode_icon, true, cx)
                    .tooltip(Tooltip::text(mode_tooltip, mode_shortcut))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        let replace = !this.replace_mode;
                        this.open(replace, window, cx)
                    })),
            ))
            .child(field)
            .child(Self::beside_field(
                status_text(status, error, cx)
                    .id("find-status")
                    .debug_selector(|| "find-status".into())
                    .pl(px(6.0))
                    .pr(px(2.0)),
            ))
            .child(Self::beside_field(previous))
            .child(Self::beside_field(next))
            .child(Self::beside_field(in_selection))
            .child(div().flex_1())
            .child(Self::beside_field(
                small_icon_button("find-close", IconName::CLOSE, true, cx)
                    .tooltip(Tooltip::text("Close", field_shortcut(&Close)))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                    ),
            ))
    }

    fn render_replace_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, current) = self.counts(cx);
        let ready = self.results_current(cx);
        let can_replace = ready && current.is_some();
        let can_replace_all = ready && count > 0;
        let excluded = self.current_excluded(cx);
        let field_focus = self.replace.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &field_focus, window).map(SharedString::from)
        };
        let new_line = self.new_line_button("find-replace-new-line", &self.replace, window, cx);
        let field = self
            .field_frame("find-replace", &self.replace, None, window, cx)
            .key_context("FindReplace")
            .child(self.field_trailing().child(new_line));
        div()
            .flex()
            .flex_row()
            .items_start()
            .h(Self::row_height(&self.replace, window, cx))
            .px(px(4.0))
            .py(px((ROW_HEIGHT - FIELD_HEIGHT) / 2.0))
            .gap(px(4.0))
            // Room of the mode button above.
            .child(div().flex_none().w(px(SMALL_BUTTON)))
            .child(field)
            .child(div().w(px(6.0)))
            .child(Self::beside_field(
                text_button("find-replace-current", "Replace", can_replace, cx)
                    .tooltip(Tooltip::text("Replace", shortcut(&ReplaceCurrent)))
                    .when(can_replace, |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.replace_current(window, cx)
                        }))
                    }),
            ))
            .child(Self::beside_field(
                text_button("find-replace-all", "Replace All", can_replace_all, cx)
                    .tooltip(Tooltip::text("Replace All", shortcut(&ReplaceAll)))
                    .when(can_replace_all, |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.replace_all(window, cx)
                        }))
                    }),
            ))
            .child(Self::beside_field(
                text_button(
                    "find-exclude",
                    if excluded { "Include" } else { "Exclude" },
                    current.is_some(),
                    cx,
                )
                .tooltip(Tooltip::text(
                    if excluded { "Include" } else { "Exclude" },
                    shortcut(&Exclude),
                ))
                .when(current.is_some(), |d| {
                    d.on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.exclude(window, cx)),
                    )
                }),
            ))
    }

    fn render_history(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let h = self.history.as_ref()?;
        // Below the field it belongs to.
        let query_row = Self::row_height(&self.query, window, cx);
        let gap = px((ROW_HEIGHT - FIELD_HEIGHT) / 2.0);
        let top = match h.field {
            Field::Query => query_row - gap + px(2.0),
            Field::Replace => {
                query_row + gap + Self::frame_height(&self.replace, window, cx) + px(2.0)
            }
        };
        let theme = cx.theme();
        let selected_bg = theme.color(UiRole::Selection);
        let selected_text = theme.color(UiRole::SelectionText);
        let hover = theme.color(UiRole::Hover);
        let rows: Vec<_> = h
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let selected = i == h.selected;
                div()
                    .id(("find-history-item", i))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(4.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(selected, |d| d.bg(selected_bg).text_color(selected_text))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(one_line(item))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.history_pick(Some(i), window, cx)
                    }))
            })
            .collect();
        Some(deferred(
            div()
                .id("find-history")
                .debug_selector(|| "find-history".into())
                .key_context("FindHistory")
                .track_focus(&self.history_focus)
                .absolute()
                .top(top)
                .left(px(FIELD_LEFT))
                .w(relative(FIELD_WIDTH))
                .min_w(px(MIN_FIELD_WIDTH))
                .max_h(px(260.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .p(px(4.0))
                .rounded(theme.metric(themes::Metric::CornerRadius) * 2.0)
                .border_1()
                .border_color(theme.color(UiRole::PopupBorder))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow(ui::components::popup_shadow())
                .children(rows),
        ))
    }
}

/// The text of a snapshot and the matches of `matcher` in it: char ranges,
/// the lines they start on, and whether the limit cut them off.
type Searched = (Arc<String>, Vec<Range<usize>>, Vec<usize>, bool);

/// The search itself, on the UI thread for a small text or on a background
/// thread: the snapshot's text (once per buffer version), the matches, and
/// their char offsets and lines.
fn run_search(
    cached: Option<Arc<String>>,
    rope: text::Rope,
    matcher: &Matcher,
    byte_scope: Option<Range<usize>>,
    cancel: &CancelToken,
) -> Result<Searched, Cancelled> {
    let text = cached.unwrap_or_else(|| {
        let mut text = String::with_capacity(rope.len_bytes());
        for chunk in rope.chunks() {
            text.push_str(chunk);
        }
        Arc::new(text)
    });
    let scope = byte_scope.unwrap_or(0..text.len());
    let found = matcher.find_all(&text, scope, MATCH_LIMIT, cancel)?;
    let mut ranges = found.ranges;
    let lines = match_lines(&text, &ranges);
    to_char_ranges(&text, &mut ranges);
    Ok((text, ranges, lines, found.truncated))
}

/// What a search found, with what it is for.
struct Found {
    reason: Reason,
    version: u64,
    applied: Applied,
    ranges: Vec<Range<usize>>,
    lines: Vec<usize>,
    truncated: bool,
}

/// A text over several lines shown on one line (hints, history items).
fn one_line(text: &str) -> String {
    text.replace('\n', " \u{21b5} ")
}

/// "0 results", "1 result", "12 results".
fn results(count: usize) -> String {
    match count {
        1 => "1 result".into(),
        n => format!("{n} results"),
    }
}

/// Moves a char range through applied edits (in the order applied): edits
/// before it shift it, edits inside it grow or shrink it.
fn map_range(range: Range<usize>, edits: &[AppliedEdit]) -> Range<usize> {
    let mut r = range;
    for e in edits {
        let map = |p: usize, is_end: bool| -> usize {
            if p <= e.start {
                p
            } else if p >= e.old_end {
                (p as isize + e.new_end as isize - e.old_end as isize) as usize
            } else if is_end {
                e.new_end
            } else {
                e.start
            }
        };
        let start = map(r.start, false);
        let end = map(r.end, true).max(start);
        r = start..end;
    }
    r
}

/// The match that becomes current after a search: the one that covers the
/// previous current match (the query grew or the text changed around it),
/// else the one at the caret, else the first in the selection, else the
/// first after the caret, else the first.
fn pick_current(
    matches: &[Range<usize>],
    old: Option<Range<usize>>,
    caret: usize,
    selection: Range<usize>,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let touches = |a: &Range<usize>, b: &Range<usize>| a.start.max(b.start) <= a.end.min(b.end);
    if let Some(old) = &old {
        let mut found = None;
        let from = matches.partition_point(|m| m.end < old.start);
        for (i, m) in matches.iter().enumerate().skip(from) {
            if m.start > old.end {
                break;
            }
            if touches(m, old) {
                found = Some(i);
            }
            if m.start == old.start {
                break;
            }
        }
        if found.is_some() {
            return found;
        }
    } else {
        let i = matches.partition_point(|m| m.end < caret);
        if matches.get(i).is_some_and(|m| m.start <= caret) {
            return Some(i);
        }
        if !selection.is_empty() {
            let i = matches.partition_point(|m| m.start < selection.start);
            if matches.get(i).is_some_and(|m| m.end <= selection.end) {
                return Some(i);
            }
        }
    }
    let i = matches.partition_point(|m| m.start < caret);
    Some(if i < matches.len() { i } else { 0 })
}

/// The match after or before `from` (the caret, or the current match when
/// searching within a selection); `None` at the end.
fn neighbour(
    matches: &[Range<usize>],
    current: Option<usize>,
    from: Range<usize>,
    direction: Direction,
) -> Option<usize> {
    match direction {
        Direction::Forward => {
            // The first match from the caret (the end of the current one)
            // on; an empty current match there is not a move.
            let mut i = matches.partition_point(|m| m.start < from.end);
            if Some(i) == current {
                i += 1;
            }
            (i < matches.len()).then_some(i)
        }
        Direction::Backward => {
            // Strictly before the caret; within a selection, ending where
            // the current match starts at the latest.
            let n = if from.is_empty() {
                matches.partition_point(|m| m.end < from.start)
            } else {
                matches.partition_point(|m| m.end <= from.start)
            };
            let i = n.checked_sub(1);
            if i.is_some() && i == current {
                i.and_then(|i| i.checked_sub(1))
            } else {
                i
            }
        }
    }
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.color(UiRole::SearchField);
        let border = theme.color(UiRole::SearchFieldBorder);
        let el = div()
            .id("find-bar")
            .debug_selector(|| "find-bar".into())
            .key_context("FindBar")
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .bg(background)
            .border_b_1()
            .border_color(border)
            .child(self.render_query_row(window, cx))
            .when(self.replace_mode, |d| {
                d.child(self.render_replace_row(window, cx))
            })
            .children(self.render_history(window, cx));
        Self::bind_actions(el, cx)
    }
}

macro_rules! find_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl FindBar {
            /// The bar's own action handlers (the focus is in the bar).
            fn bind_actions<E: InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Adds handlers to `el` that send the find commands to `bar`; the
        /// host of an editor uses this so that Find, Find Next and the
        /// other commands work while the editor has the focus.
        pub fn forward_actions<E: InteractiveElement>(el: E, bar: Entity<FindBar>) -> E {
            el $(.on_action({
                let bar = bar.clone();
                move |a: &$action, window, cx| bar.update(cx, |b, cx| b.$method(a, window, cx))
            }))*
        }
    };
}

find_actions! {
    Open => open_action,
    OpenReplace => open_replace_action,
    Next => next_action,
    Previous => previous_action,
    Search => search_action,
    Close => close_action,
    ToggleCase => toggle_case_action,
    ToggleWords => toggle_words_action,
    ToggleRegex => toggle_regex_action,
    ToggleInSelection => toggle_in_selection_action,
    ReplaceCurrent => replace_current_action,
    ReplaceAll => replace_all_action,
    Exclude => exclude_action,
    FocusEditor => focus_editor_action,
    SwitchField => switch_field_action,
    ShowHistory => show_history_action,
    NewLine => new_line_action,
    Down => down_action,
    Up => up_action,
    HistoryUp => history_up_action,
    HistoryDown => history_down_action,
    HistoryPick => history_pick_action,
    HistoryClose => history_close_action,
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // expected ranges
mod tests {
    use super::*;

    #[test]
    fn current_match_follows_the_query_and_the_caret() {
        let m = vec![0..3, 10..13, 20..23];
        // At the caret (inclusive ends), else after it, else the first.
        assert_eq!(pick_current(&m, None, 11, 11..11), Some(1));
        assert_eq!(pick_current(&m, None, 13, 10..13), Some(1));
        assert_eq!(pick_current(&m, None, 15, 15..15), Some(2));
        assert_eq!(pick_current(&m, None, 25, 25..25), Some(0));
        // The previous current match grew: it stays current.
        assert_eq!(
            pick_current(&[0..4, 10..14], Some(10..13), 13, 10..13),
            Some(1)
        );
        // Nothing covers it any more: the next one after the caret.
        assert_eq!(
            pick_current(&[0..4, 20..24], Some(10..13), 13, 10..13),
            Some(1)
        );
        assert_eq!(pick_current(&[], None, 0, 0..0), None);
    }

    #[test]
    fn neighbours_in_both_directions() {
        let m = vec![0..3, 10..13, 20..23];
        // The current match is selected with the caret at its end.
        assert_eq!(neighbour(&m, Some(1), 13..13, Direction::Forward), Some(2));
        assert_eq!(neighbour(&m, Some(2), 23..23, Direction::Forward), None);
        assert_eq!(neighbour(&m, Some(1), 13..13, Direction::Backward), Some(0));
        assert_eq!(neighbour(&m, Some(0), 3..3, Direction::Backward), None);
        // A caret between matches.
        assert_eq!(neighbour(&m, None, 5..5, Direction::Forward), Some(1));
        assert_eq!(neighbour(&m, None, 5..5, Direction::Backward), Some(0));
        // Empty matches (`^`) do not get stuck.
        let e = vec![0..0, 4..4, 8..8];
        assert_eq!(neighbour(&e, Some(1), 4..4, Direction::Forward), Some(2));
        assert_eq!(neighbour(&e, Some(1), 4..4, Direction::Backward), Some(0));
        // Within a selection the current match is the reference.
        assert_eq!(neighbour(&m, Some(1), 10..13, Direction::Forward), Some(2));
        assert_eq!(neighbour(&m, Some(1), 10..13, Direction::Backward), Some(0));
    }

    #[test]
    fn ranges_move_with_edits() {
        let edit = |start, old_end, new_end| AppliedEdit {
            start,
            old_end,
            new_end,
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 0,
            start_position: (0, 0),
            old_end_position: (0, 0),
            new_end_position: (0, 0),
        };
        // Before: shifted; inside: grows; after: unchanged.
        assert_eq!(map_range(10..20, &[edit(2, 4, 7)]), 13..23);
        assert_eq!(map_range(10..20, &[edit(12, 15, 13)]), 10..18);
        assert_eq!(map_range(10..20, &[edit(20, 22, 30)]), 10..20);
        // A replacement at the start stays inside.
        assert_eq!(map_range(10..20, &[edit(10, 13, 18)]), 10..25);
        // Deleting across the end.
        assert_eq!(map_range(10..20, &[edit(15, 25, 15)]), 10..15);
    }

    #[test]
    fn histories_keep_the_newest_first_once() {
        let mut h = Vec::new();
        remember(&mut h, "a");
        remember(&mut h, "b");
        remember(&mut h, "a");
        remember(&mut h, "");
        assert_eq!(h, ["a", "b"]);
        for i in 0..40 {
            remember(&mut h, &i.to_string());
        }
        assert_eq!(h.len(), HISTORY_SIZE);
        assert_eq!(h[0], "39");
    }
}
