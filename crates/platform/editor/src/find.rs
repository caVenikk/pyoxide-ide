//! The find bar of an editor (Find and Replace in the file): a search field
//! with the Match Case, Words and Regex options, the match counter and the
//! previous and next buttons, shown at the top of the editor.
//!
//! Matches are found by the `search` crate on a snapshot of the editor's
//! text off the UI thread; every change of the query starts a new search
//! and cancels the one still running. The editor paints the matches
//! ([`SearchHighlights`]); the current match is selected. The behaviour
//! follows the reference IDE (tests/scenarios/editor/find-replace.md): the
//! selected text or the last search fills the field, typing searches
//! incrementally, Enter and Shift+Enter move through the matches, moving
//! past the last match first says so and then starts over, Escape closes
//! the bar and keeps the current match selected.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Task, Window, div, prelude::*, px, relative,
};
use search::{CancelToken, Cancelled, Query, QueryError, SearchOptions, to_char_ranges};
use serde_json::json;
use text::{Buffer, SelectionSet};
use themes::UiRole;
use ui::components::{Tooltip, input_frame, small_icon_button, status_text, toggle_button};
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
    } => "find.search", "Find Text",
        "Opens the find bar and searches for a text; options that are given are switched on or off.",
        r#"{"text": "def", "case_sensitive": false, "whole_words": false, "regex": false}"#;
}

ui::ui_commands! {
    BAR_COMMANDS, scope = "FindBar";
    /// Close the find bar.
    Close => "find.close", "Close",
        "Closes the find bar, removes the highlighting of the matches and keeps the current match selected.";
    /// Match case on or off.
    ToggleCase => "find.toggle-case", "Match Case", "Switches case-sensitive matching on or off.";
    /// Whole words on or off.
    ToggleWords => "find.toggle-words", "Words", "Switches whole-word matching on or off.";
    /// Regular expressions on or off.
    ToggleRegex => "find.toggle-regex", "Regex", "Switches regular expression matching on or off.";
}

/// Most matches one search collects; the counter shows `1000000+` beyond.
pub const MATCH_LIMIT: usize = 1_000_000;
/// Height of a row of the bar and of its fields.
const ROW_HEIGHT: f32 = 34.0;
const FIELD_HEIGHT: f32 = 28.0;
/// Share of the bar's width taken by the fields, and their smallest width.
const FIELD_WIDTH: f32 = 0.4;
const MIN_FIELD_WIDTH: f32 = 240.0;

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
}

impl Global for LastSearch {}

/// The last search of the session.
pub fn last_search(cx: &App) -> LastSearch {
    cx.try_global::<LastSearch>().cloned().unwrap_or_default()
}

fn update_last_search(cx: &mut App, f: impl FnOnce(&mut LastSearch)) {
    f(cx.default_global::<LastSearch>());
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
    /// The bar is closed: Find Next or Previous with the last search.
    Again(Direction),
}

/// The find bar of one editor.
pub struct FindBar {
    editor: Entity<Editor>,
    query: Entity<Editor>,
    replace: Entity<Editor>,
    open: bool,
    replace_mode: bool,
    options: SearchOptions,
    /// Search only within this char range of the editor (a selection over
    /// several lines when the bar opened).
    scope: Option<Range<usize>>,
    status: Status,
    truncated: bool,
    /// The editor's text as last searched, with its buffer version.
    snapshot: Option<(u64, Arc<String>)>,
    search_task: Option<Task<()>>,
    cancel: Option<CancelToken>,
    /// Next or Previous asked for while a search ran.
    pending: Option<Direction>,
    /// The last move found nothing this way; the next move this way starts
    /// over from the other end.
    wrap: Option<Direction>,
    /// What the last move found (or did not find), shown near the caret.
    hint: Option<SharedString>,
    /// The editor's selections when the bar opened, restored when the
    /// query is cleared.
    initial: Option<SelectionSet>,
    _subscriptions: Vec<Subscription>,
}

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
                e.set_placeholder(Some(placeholder.into()));
                e
            })
        };
        let query = field("Search", window, cx);
        let replace = field("Replace", window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event, window, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    this.query_edited(window, cx);
                }
            }),
            cx.subscribe_in(&replace, window, |_, field, event, _, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    let text = field.read(cx).text();
                    update_last_search(cx, |s| s.replace = text);
                }
            }),
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                EditorEvent::Edited(_) if this.open => this.search(Reason::Edited, window, cx),
                EditorEvent::SelectionsChanged => this.editor_selection_changed(cx),
                _ => {}
            }),
        ];
        Self {
            editor,
            query,
            replace,
            open: false,
            replace_mode: false,
            options: SearchOptions::default(),
            scope: None,
            status: Status::Empty,
            truncated: false,
            snapshot: None,
            search_task: None,
            cancel: None,
            pending: None,
            wrap: None,
            hint: None,
            initial: None,
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

    /// What the last move said ("not found, press ... to search from the
    /// top"), shown near the caret until the next key.
    pub fn hint(&self) -> Option<&SharedString> {
        self.hint.as_ref()
    }

    pub fn clear_hint(&mut self, cx: &mut Context<Self>) {
        if self.hint.take().is_some() {
            cx.notify();
        }
    }

    /// A search is running.
    pub fn is_searching(&self) -> bool {
        self.search_task.is_some()
    }

    /// Number of matches and the one-based index of the current match.
    pub fn counts(&self, cx: &App) -> (usize, Option<usize>) {
        self.editor
            .read(cx)
            .search_highlights()
            .map_or((0, None), |h| (h.matches.len(), h.current.map(|i| i + 1)))
    }

    /// The counter text and whether it reports a problem (nothing found, a
    /// bad pattern).
    pub fn status(&self, cx: &App) -> (String, bool) {
        let (count, current) = self.counts(cx);
        match self.status {
            Status::Empty => (results(0), false),
            Status::BadPattern => ("Bad pattern".into(), true),
            Status::Vague => ("Vague pattern".into(), false),
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
        let current_range = editor
            .search_highlights()
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
            "truncated": self.truncated,
            "searching": self.is_searching(),
            "status": status,
            "error": error,
            "hint": self.hint.as_ref().map(|h| h.to_string()),
        })
    }

    // ---- opening and closing ---------------------------------------------

    /// Opens the bar (or brings the focus back to it): a single-line
    /// selection becomes the query, a selection over several lines limits
    /// the search to it; otherwise the field keeps the last search. The
    /// field text is selected so that typing replaces it.
    pub fn open(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let last = last_search(cx);
        let was_open = self.open;
        if !was_open {
            self.options = last.options;
        }
        self.replace_mode = replace;
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
        if text.contains('\n') {
            text.clear();
        }
        self.scope = None;
        // The selected current match of an open bar is not a new query.
        let selection_is_match = was_open && current.as_ref() == Some(&selection);
        if !selected.is_empty() && !selection_is_match {
            if selected.contains('\n') {
                self.scope = Some(selection);
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
        self.open = true;
        self.wrap = None;
        self.hint = None;
        self.pending = None;
        self.initial = Some(self.editor.read(cx).selections().clone());
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
        self.pending = None;
        self.hint = None;
        self.snapshot = None;
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
            self.open(false, window, cx);
        }
        self.set_options(options, cx);
        self.query.update(cx, |f, cx| f.set_text_selected(text, cx));
        self.query_edited(window, cx);
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

    /// Starts a search on a snapshot of the editor's text; the result is
    /// applied when it comes back, unless a newer search replaced it.
    fn search(&mut self, reason: Reason, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_search();
        let query = match reason {
            Reason::Again(_) => {
                let last = last_search(cx);
                Query::new(last.text, last.options)
            }
            _ => Query::new(self.query.read(cx).text(), self.options),
        };
        let matcher = match query.compile() {
            Ok(matcher) => matcher,
            Err(e) => {
                if !matches!(reason, Reason::Again(_)) {
                    self.status = match e {
                        QueryError::Empty => Status::Empty,
                        QueryError::Vague => Status::Vague,
                        QueryError::InvalidRegex(_) => Status::BadPattern,
                    };
                    self.truncated = false;
                    let restore = (e == QueryError::Empty && reason == Reason::Query)
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
        let scope = match reason {
            Reason::Again(_) => None,
            _ => self.scope.clone(),
        }
        .map(|r| {
            let end = r.end.min(rope.len_chars());
            rope.char_to_byte(r.start.min(end))..rope.char_to_byte(end)
        });
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        if !matches!(reason, Reason::Again(_)) {
            self.status = Status::Searching;
        }
        let work = cx.background_spawn(async move {
            let text = cached.unwrap_or_else(|| {
                let mut text = String::with_capacity(rope.len_bytes());
                for chunk in rope.chunks() {
                    text.push_str(chunk);
                }
                Arc::new(text)
            });
            let scope = scope.unwrap_or(0..text.len());
            let found = matcher.find_all(&text, scope, MATCH_LIMIT, &cancel)?;
            let mut ranges = found.ranges;
            to_char_ranges(&text, &mut ranges);
            Ok::<_, Cancelled>((text, ranges, found.truncated))
        });
        self.search_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.search_task = None;
                this.cancel = None;
                if let Ok((text, ranges, truncated)) = result {
                    this.search_done(reason, version, text, ranges, truncated, window, cx);
                }
            });
        }));
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn search_done(
        &mut self,
        reason: Reason,
        version: u64,
        text: Arc<String>,
        matches: Vec<Range<usize>>,
        truncated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.read(cx).buffer().version() != version {
            // The text changed while the search ran.
            self.search(reason, window, cx);
            return;
        }
        self.snapshot = Some((version, text));
        if let Reason::Again(direction) = reason {
            self.move_without_bar(direction, &matches, window, cx);
            return;
        }
        if !self.open {
            return;
        }
        self.status = Status::Done;
        self.truncated = truncated;
        let (old, caret, selection) = {
            let e = self.editor.read(cx);
            let s = e.selections().primary();
            (
                e.search_highlights().and_then(|h| h.current_range()),
                s.head,
                s.range(),
            )
        };
        let current = pick_current(&matches, old, caret, selection);
        let target = current.map(|i| matches[i].clone());
        let in_selection = self.scope.is_some();
        let select = reason == Reason::Query;
        let highlights = SearchHighlights {
            matches,
            current,
            current_background: in_selection,
            excluded: Default::default(),
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
                self.editor.update(cx, |e, cx| {
                    e.set_search_current(Some(i), cx);
                    let range = e.search_highlights().map(|h| h.matches[i].clone());
                    if let Some(range) = range {
                        if in_selection {
                            e.reveal(range.start, cx);
                        } else {
                            e.select_and_reveal(range, cx);
                        }
                    }
                });
            }
            None => self.not_found(direction, true, window, cx),
        }
        cx.notify();
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
        let text = match self.open {
            true => self.query.read(cx).text(),
            false => last_search(cx).text,
        };
        let mut message = format!("\"{text}\" not found");
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
    }

    /// The caret moved: a move that went nowhere no longer wraps, unless the
    /// move was the find bar's own.
    fn editor_selection_changed(&mut self, cx: &mut Context<Self>) {
        if self.wrap.is_none() && self.hint.is_none() {
            return;
        }
        let e = self.editor.read(cx);
        let selection = e.selections().primary().range();
        let ours = e
            .search_highlights()
            .and_then(|h| h.current_range())
            .is_some_and(|r| r == selection);
        if !ours {
            self.wrap = None;
            self.hint = None;
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
        self.search_for(&a.text, options, window, cx);
    }

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn toggle_case(&mut self, _: &ToggleCase, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.case_sensitive = !o.case_sensitive, window, cx);
    }

    fn toggle_words(&mut self, _: &ToggleWords, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.whole_words = !o.whole_words, window, cx);
    }

    fn toggle_regex(&mut self, _: &ToggleRegex, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.regex = !o.regex, window, cx);
    }

    fn toggle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let replace = !self.replace_mode;
        self.open(replace, window, cx);
    }

    // ---- drawing --------------------------------------------------------------

    fn render_query_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = self.query.focus_handle(cx).is_focused(window);
        let (status, error) = self.status(cx);
        let (count, _) = self.counts(cx);
        let field_error = self.query.read(cx).is_error();
        let editor_focus = self.editor.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &editor_focus, window).map(SharedString::from)
        };
        let field_focus = self.query.focus_handle(cx);
        let option_shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &field_focus, window).map(SharedString::from)
        };
        let toggles = div()
            .flex()
            .flex_row()
            .flex_none()
            .gap(px(2.0))
            .pr(px(3.0))
            .child(
                toggle_button(
                    "find-match-case",
                    IconName::MATCH_CASE,
                    self.options.case_sensitive,
                    cx,
                )
                .tooltip(Tooltip::text("Match Case", option_shortcut(&ToggleCase)))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_case(&ToggleCase, window, cx)
                })),
            )
            .child(
                toggle_button("find-words", IconName::WORDS, self.options.whole_words, cx)
                    .tooltip(Tooltip::text("Words", option_shortcut(&ToggleWords)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_words(&ToggleWords, window, cx)
                    })),
            )
            .child(
                toggle_button("find-regex", IconName::REGEX, self.options.regex, cx)
                    .tooltip(Tooltip::text("Regex", option_shortcut(&ToggleRegex)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_regex(&ToggleRegex, window, cx)
                    })),
            );
        let field = input_frame(focused, field_error, cx)
            .id("find-query")
            .debug_selector(|| "find-query".into())
            .w(relative(FIELD_WIDTH))
            .min_w(px(MIN_FIELD_WIDTH))
            .h(px(FIELD_HEIGHT))
            .child(
                div()
                    .flex_none()
                    .pl(px(6.0))
                    .pr(px(4.0))
                    .child(Icon::new(IconName::SEARCH_HISTORY)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(theme.editor_size() * theme.editor_line_spacing() + px(6.0))
                    .overflow_hidden()
                    .child(self.query.clone()),
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
        let mode_icon = if self.replace_mode {
            IconName::CHEVRON_DOWN
        } else {
            IconName::CHEVRON_RIGHT
        };
        let mode_tooltip = if self.replace_mode {
            "Switch to Find"
        } else {
            "Switch to Replace"
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW_HEIGHT))
            .px(px(4.0))
            .gap(px(4.0))
            .child(
                small_icon_button("find-mode", mode_icon, true, cx)
                    .tooltip(Tooltip::text(mode_tooltip, None))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.toggle_mode(window, cx)
                        }),
                    ),
            )
            .child(field)
            .child(
                status_text(status, error, cx)
                    .id("find-status")
                    .debug_selector(|| "find-status".into())
                    .pl(px(6.0))
                    .pr(px(2.0)),
            )
            .child(previous)
            .child(next)
            .child(div().flex_1())
            .child(
                small_icon_button("find-close", IconName::CLOSE, true, cx)
                    .tooltip(Tooltip::text("Close", None))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                    ),
            )
    }

    fn render_replace_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = self.replace.focus_handle(cx).is_focused(window);
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW_HEIGHT))
            .px(px(4.0))
            .gap(px(4.0))
            // Room of the mode button above.
            .child(div().flex_none().w(px(ui::components::SMALL_BUTTON)))
            .child(
                input_frame(focused, false, cx)
                    .id("find-replace")
                    .debug_selector(|| "find-replace".into())
                    .w(relative(FIELD_WIDTH))
                    .min_w(px(MIN_FIELD_WIDTH))
                    .h(px(FIELD_HEIGHT))
                    .pl(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(theme.editor_size() * theme.editor_line_spacing() + px(6.0))
                            .overflow_hidden()
                            .child(self.replace.clone()),
                    ),
            )
    }
}

/// "0 results", "1 result", "12 results".
fn results(count: usize) -> String {
    match count {
        1 => "1 result".into(),
        n => format!("{n} results"),
    }
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
        let background = theme.color(UiRole::EditorBackground);
        let border = theme.color(UiRole::Border);
        let el = div()
            .id("find-bar")
            .debug_selector(|| "find-bar".into())
            .key_context("FindBar")
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
            });
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
    ToggleCase => toggle_case,
    ToggleWords => toggle_words,
    ToggleRegex => toggle_regex,
}

#[cfg(test)]
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
}
