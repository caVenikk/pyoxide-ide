//! The find bar of a session (`TerminalSearchController`,
//! `TerminalSearchSession`, IntelliJ Community, Apache-2.0): a panel over
//! the top right of the session with the query field (a clear button, Match
//! Case and Regex inside it), the count "n/m", Previous and Next, and
//! Close. There are no Words and no Replace. The matches are marked on
//! the screen (`BLOCK_TERMINAL_SEARCH_ENTRY`, the current one
//! `BLOCK_TERMINAL_CURRENT_SEARCH_ENTRY`) and counted on a background
//! thread, in the whole output.
//!
//! Enter goes to the next match, Shift+Enter to the previous one, Escape
//! closes the bar and gives the focus back to the session, whose Escape
//! then goes to the shell.

use std::time::{Duration, Instant};

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, Focusable as _, IntoElement, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use serde_json::{Value, json};
use terminal::{MatchCount, SearchDirection, SearchQuery};
use themes::UiRole;
use ui::components::{Tooltip, input_frame, small_icon_button, status_text, toggle_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::commands::{Find, FindClose, FindNext, FindPrevious, FindToggleCase, FindToggleRegex};
use crate::view::SessionView;

/// The key context of the bar (its field is an editor inside it).
pub const FIND_CONTEXT: &str = "TerminalFind";
/// The bar's width (`TerminalUi.searchComponentWidth`) and the field's.
const BAR_WIDTH: f32 = 488.0;
const FIELD_WIDTH: f32 = 280.0;
const FIELD_HEIGHT: f32 = 28.0;
/// Space above and right of the bar, over the output.
const BAR_TOP: f32 = 7.0;
const BAR_RIGHT: f32 = 6.0;
/// The most matches counted.
const COUNT_LIMIT: usize = 9999;
/// The shortest time between two counts while output changes.
const RECOUNT: Duration = Duration::from_millis(300);

/// What the bar remembers between openings.
#[derive(Clone, Debug, Default)]
pub(crate) struct FindPrefs {
    pub match_case: bool,
    pub regex: bool,
    pub last: String,
}

/// An open find bar.
pub(crate) struct FindState {
    pub field: Entity<Editor>,
    pub query: SearchQuery,
    /// The pattern is not a regular expression.
    pub invalid: Option<String>,
    pub count: MatchCount,
    /// The query was counted at least once.
    pub counted: bool,
    counting: Option<Task<()>>,
    /// The query or the output changed while a count ran.
    dirty: bool,
    last_count: Option<Instant>,
    /// A count waiting for the shortest time between counts to pass.
    later: Option<Task<()>>,
    epoch: u64,
    _subscription: Subscription,
}

impl SessionView {
    /// The query a new bar starts with: the selected text when it is on
    /// one line, else the last query.
    fn find_start_text(&self) -> String {
        match self.session().and_then(|s| s.selection_text()) {
            Some(text) if !text.contains('\n') && !text.trim().is_empty() => text,
            _ => self.find_prefs.last.clone(),
        }
    }

    /// Opens the bar (`Find`): with the focus in its field and the query
    /// selected. `text` is the query, searched at once.
    pub fn open_find(&mut self, text: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let start = match text {
            Some(text) => text.to_owned(),
            None if self.find.is_some() => self
                .find
                .as_ref()
                .map(|f| f.query.text.clone())
                .unwrap_or_default(),
            None => self.find_start_text(),
        };
        let fresh = self.find.is_none();
        if fresh {
            let field = editor::field::new(&start, Some("Search".into()), true, window, cx);
            let subscription = cx.subscribe(&field, |this, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    this.find_edited(cx);
                }
            });
            let prefs = &self.find_prefs;
            self.find = Some(FindState {
                field,
                query: SearchQuery {
                    text: start.clone(),
                    match_case: prefs.match_case,
                    regex: prefs.regex,
                },
                invalid: None,
                count: MatchCount::default(),
                counted: false,
                counting: None,
                dirty: false,
                last_count: None,
                later: None,
                epoch: 0,
                _subscription: subscription,
            });
        }
        if let Some(find) = &mut self.find {
            find.query.text = start.clone();
            let field = find.field.clone();
            field.update(cx, |e, cx| e.set_text_selected(&start, cx));
            window.focus(&field.focus_handle(cx));
        }
        self.find_search(true, cx);
        cx.notify();
    }

    /// The query field was edited.
    fn find_edited(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let text = find.field.read(cx).text();
        if text == find.query.text {
            return;
        }
        find.query.text = text;
        self.find_search(true, cx);
    }

    /// Sets the query in the session; with `jump` goes to the first match.
    fn find_search(&mut self, jump: bool, cx: &mut Context<Self>) {
        let (Some(find), Some(session)) = (&mut self.find, self.session.as_ref()) else {
            return;
        };
        self.find_prefs.last = find.query.text.clone();
        self.find_prefs.match_case = find.query.match_case;
        self.find_prefs.regex = find.query.regex;
        find.count = MatchCount::default();
        find.counted = false;
        match session.set_search(Some(&find.query)) {
            Ok(()) => {
                find.invalid = None;
                if jump && !find.query.text.is_empty() {
                    session.find(SearchDirection::Next);
                }
            }
            Err(e) => {
                find.invalid = Some(e.to_string());
                let _ = session.set_search(None);
            }
        }
        self.recount(cx);
        self.show_now(cx);
    }

    /// Counts the matches in the whole output on a background thread.
    fn recount(&mut self, cx: &mut Context<Self>) {
        let (Some(find), Some(session)) = (&mut self.find, self.session.as_ref()) else {
            return;
        };
        if find.query.text.is_empty() || find.invalid.is_some() {
            find.count = MatchCount::default();
            find.counted = true;
            find.counting = None;
            return;
        }
        if find.counting.is_some() {
            find.dirty = true;
            return;
        }
        find.dirty = false;
        find.last_count = Some(Instant::now());
        find.epoch += 1;
        let epoch = find.epoch;
        let screen = session.screen();
        let work = cx.background_spawn(async move { screen.count_matches(COUNT_LIMIT) });
        find.counting = Some(cx.spawn(async move |this, cx| {
            let count = work.await;
            let _ = this.update(cx, |v, cx| {
                let Some(find) = &mut v.find else {
                    return;
                };
                find.counting = None;
                if find.epoch == epoch {
                    find.count = count;
                    find.counted = true;
                }
                let again = find.dirty;
                cx.notify();
                if again {
                    v.recount(cx);
                }
            });
        }));
    }

    /// New output arrived while the bar is open: the count follows, at most
    /// every [`RECOUNT`].
    pub(crate) fn find_output_changed(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        if find.query.text.is_empty() || find.invalid.is_some() {
            return;
        }
        if find.counting.is_some() {
            find.dirty = true;
            return;
        }
        match find.last_count.map(|t| t.elapsed()) {
            Some(elapsed) if elapsed < RECOUNT => {
                // The output may stop before the time has passed: count
                // once it has.
                if find.later.is_none() {
                    let wait = RECOUNT - elapsed;
                    find.later = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(wait).await;
                        let _ = this.update(cx, |v, cx| {
                            if let Some(find) = &mut v.find {
                                find.later = None;
                            }
                            v.recount(cx);
                        });
                    }));
                }
            }
            _ => self.recount(cx),
        }
    }

    /// Goes to the next or previous match (opening the bar when it is
    /// closed).
    pub fn find_step(
        &mut self,
        direction: SearchDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find.is_none() {
            self.open_find(None, window, cx);
        }
        let (Some(find), Some(session)) = (&mut self.find, self.session.as_ref()) else {
            return;
        };
        if find.query.text.is_empty() || find.invalid.is_some() {
            return;
        }
        session.find(direction);
        // The new current match is counted again (its index changes).
        find.dirty = false;
        self.recount(cx);
        self.show_now(cx);
    }

    /// Closes the bar; the focus goes back to the session.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_none() {
            return;
        }
        if let Some(session) = &self.session {
            let _ = session.set_search(None);
        }
        window.focus(&self.focus);
        self.show_now(cx);
    }

    fn toggle_find_option(&mut self, case: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        if case {
            find.query.match_case = !find.query.match_case;
        } else {
            find.query.regex = !find.query.regex;
        }
        self.find_search(true, cx);
    }

    pub(crate) fn find_action(
        &mut self,
        action: &Find,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_find(action.text.as_deref(), window, cx);
    }

    pub(crate) fn find_next_action(
        &mut self,
        _: &FindNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_step(SearchDirection::Next, window, cx);
    }

    pub(crate) fn find_previous_action(
        &mut self,
        _: &FindPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_step(SearchDirection::Previous, window, cx);
    }

    pub(crate) fn find_close_action(
        &mut self,
        _: &FindClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_find(window, cx);
    }

    pub(crate) fn find_case_action(
        &mut self,
        _: &FindToggleCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_find_option(true, cx);
    }

    pub(crate) fn find_regex_action(
        &mut self,
        _: &FindToggleRegex,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_find_option(false, cx);
    }

    /// What the counter says: "n/m", "m+" when counting stopped at the
    /// limit, "0 results" without a match.
    fn find_status(&self) -> (String, bool) {
        let Some(find) = &self.find else {
            return (String::new(), false);
        };
        if let Some(error) = &find.invalid {
            let _ = error;
            return ("Bad pattern".into(), true);
        }
        if find.query.text.is_empty() {
            return (String::new(), false);
        }
        let total = find.count.total;
        if total == 0 {
            return ("0 results".into(), find.counted);
        }
        let plus = if total >= COUNT_LIMIT { "+" } else { "" };
        match find.count.current {
            Some(current) => (format!("{}/{total}{plus}", current + 1), false),
            None => (format!("{total}{plus}"), false),
        }
    }

    /// The bar's state as JSON (`terminal.state`).
    pub(crate) fn find_json(&self) -> Value {
        match &self.find {
            None => json!({ "open": false }),
            Some(find) => json!({
                "open": true,
                "query": find.query.text,
                "match_case": find.query.match_case,
                "regex": find.query.regex,
                "error": find.invalid,
                "matches": find.count.total,
                "current": find.count.current.map(|c| c + 1),
                "visible_matches": self.snapshot.matches.len(),
                "has_current_on_screen": self.snapshot.current_match.is_some(),
            }),
        }
    }

    /// The bar, over the top right of the session.
    pub(crate) fn render_find(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let (status, error) = self.find_status();
        let field = find.field.clone();
        let invalid = find.invalid.is_some() || (find.counted && error);
        let (match_case, regex) = (find.query.match_case, find.query.regex);
        let has_matches = find.count.total > 0;
        let has_text = !find.query.text.is_empty();
        let focus = field.focus_handle(cx);
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &focus, window).map(SharedString::from)
        };
        let theme = cx.theme();
        let focused = focus.is_focused(window);
        let line = editor::field_line_height_for(true, window, cx);

        let trailing =
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(2.0))
                .pr(px(3.0))
                .when(has_text, |d| {
                    d.child(
                        small_icon_button("terminal-find-clear", IconName::CLOSE_SMALL, true, cx)
                            .tooltip(Tooltip::text("Clear", None))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                if let Some(find) = &this.find {
                                    let field = find.field.clone();
                                    field.update(cx, |e, cx| e.set_text_selected("", cx));
                                    window.focus(&field.focus_handle(cx));
                                }
                            })),
                    )
                })
                .child(
                    toggle_button("terminal-find-case", IconName::MATCH_CASE, match_case, cx)
                        .tooltip(Tooltip::text("Match Case", shortcut(&FindToggleCase)))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_find_option(true, cx)
                        })),
                )
                .child(
                    toggle_button("terminal-find-regex", IconName::REGEX, regex, cx)
                        .tooltip(Tooltip::text("Regex", shortcut(&FindToggleRegex)))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_find_option(false, cx)
                        })),
                );
        let field_frame = input_frame(focused, invalid, cx)
            .id("terminal-find-field")
            .debug_selector(|| "terminal-find-field".into())
            .flex_none()
            .w(px(FIELD_WIDTH))
            .h(px(FIELD_HEIGHT))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl(px(6.0))
                    .pr(px(4.0))
                    .child(Icon::new(IconName::SEARCH_HISTORY)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(line)
                    .overflow_hidden()
                    .child(field),
            )
            .child(trailing);

        let previous = small_icon_button(
            "terminal-find-previous",
            IconName::ARROW_UP,
            has_matches,
            cx,
        )
        .tooltip(Tooltip::text(
            "Previous Occurrence",
            shortcut(&FindPrevious),
        ))
        .when(has_matches, |d| {
            d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.find_step(SearchDirection::Previous, window, cx)
            }))
        });
        let next = small_icon_button("terminal-find-next", IconName::ARROW_DOWN, has_matches, cx)
            .tooltip(Tooltip::text("Next Occurrence", shortcut(&FindNext)))
            .when(has_matches, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.find_step(SearchDirection::Next, window, cx)
                }))
            });
        let close = small_icon_button("terminal-find-close", IconName::CLOSE, true, cx)
            .tooltip(Tooltip::text("Close", shortcut(&FindClose)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_find(window, cx)));

        let bar = div()
            .id("terminal-find")
            .debug_selector(|| "terminal-find".into())
            .key_context(FIND_CONTEXT)
            .absolute()
            .top(px(BAR_TOP))
            .right(px(BAR_RIGHT))
            .w(px(BAR_WIDTH))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(5.0))
            .py(px(7.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            // A click in the bar is not a click in the output.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(field_frame)
            .child(
                status_text(status, error && find.counted, cx)
                    .id("terminal-find-status")
                    .debug_selector(|| "terminal-find-status".into())
                    .flex_none()
                    .w(px(64.0))
                    .flex()
                    .justify_center(),
            )
            .child(previous)
            .child(next)
            .child(div().flex_1())
            .child(close);
        Some(bar.into_any_element())
    }
}
