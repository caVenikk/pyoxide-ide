//! The editor entity: a buffer, its selections and the view state, plus
//! every editing and caret command.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, ClipboardEntry, ClipboardItem, Context, Div, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Render, SharedString, Styled, Subscription, Task, Window, div, point,
    px,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use text::movement::{self, WordStop};
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Point, Selection, SelectionSet};

use crate::commands::*;
use crate::decorations::{DecorationId, Decorations};
use crate::diagnostics::{Analysis, DiagnosticMark, Marks, ProblemCounts, problem_to_go};
use crate::element::{EditorElement, LayoutInfo};
use crate::highlight::Highlighter;
use crate::soft_wrap::SoftWrap;
use crate::wrap::{WrapParams, WrapRule};

/// How long the caret stays visible or hidden while blinking.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Indentation and language hints for one editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    /// Display width of a tab character.
    pub tab_size: usize,
    /// Spaces per indentation level.
    pub indent: usize,
    /// A line ending (before trailing spaces) with one of these characters
    /// gets one more level of indentation on the next line (`:` for Python).
    pub indent_after: Vec<char>,
    /// A line starting with one of these words ends its block: the next
    /// line gets one level less (`return`, `pass`, ... for Python).
    pub dedent_after: Vec<String>,
    /// Prefix of a line comment with the space after it (`"# "`).
    pub line_comment: Option<String>,
    /// Bracket pairs that are closed when the opening one is typed.
    pub brackets: Vec<(char, char)>,
    /// Quote characters that are closed when typed.
    pub quotes: Vec<char>,
    /// A third quote typed after an empty pair opens a triple-quoted string.
    pub triple_quotes: bool,
    /// Letters that may precede an opening quote as a string prefix (`f`,
    /// `r`, `b` ...); after other word characters a quote is not paired.
    pub string_prefixes: Vec<char>,
    /// Where a soft-wrapped line may wrap when no space is near (the
    /// file type's line wrap strategy).
    pub wrap_rules: Vec<WrapRule>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent: 4,
            indent_after: Vec::new(),
            dedent_after: Vec::new(),
            line_comment: None,
            brackets: vec![('(', ')'), ('[', ']'), ('{', '}')],
            quotes: Vec::new(),
            triple_quotes: false,
            string_prefixes: Vec::new(),
            wrap_rules: WrapRule::defaults(),
        }
    }
}

/// Moves a position (chars, before `edits`) to where it is after them.
/// Insertions at the position do not move it; a deletion around it pulls
/// it to the deletion's start.
fn map_through(edits: &[Edit], pos: usize) -> usize {
    let mut delta: isize = 0;
    for e in edits {
        let (start, end) = (e.range.start, e.range.end);
        if start >= pos {
            continue;
        }
        let removed = pos.min(end) - start;
        delta -= removed as isize;
        delta += e.text.chars().count() as isize;
    }
    (pos as isize + delta).max(0) as usize
}

/// Width of the leading whitespace of `text` in chars.
fn leading_ws(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// Search matches painted in the text (the find bar's results).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchHighlights {
    /// Char ranges, sorted and not overlapping.
    pub matches: Vec<Range<usize>>,
    /// Index of the current match: outlined in the caret colour.
    pub current: Option<usize>,
    /// The current match also gets the active match background (when it
    /// is not selected, as in search within a selection).
    pub current_background: bool,
    /// Matches left out of Replace All: struck through.
    pub excluded: BTreeSet<usize>,
    /// Lines with matches (sorted, each once), marked on the scrollbar.
    pub lines: Vec<usize>,
    /// The background of the matches.
    pub background: MatchBackground,
}

/// The background of [`SearchHighlights`]' matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchBackground {
    /// Text found by a search (the reference's
    /// `TEXT_SEARCH_RESULT_ATTRIBUTES`).
    #[default]
    Match,
    /// A found usage in a usage preview (`SEARCH_RESULT_ATTRIBUTES`).
    Active,
    /// Occurrences of the text at the caret
    /// (`IDENTIFIER_UNDER_CARET_ATTRIBUTES`, also used for the occurrences
    /// of the selected text).
    Occurrence,
}

/// Edits of more than this many replacements drop the highlighted matches
/// instead of moving them (the find bar searches again anyway).
const MAX_EDITS_TO_TRACK: usize = 64;

impl SearchHighlights {
    pub fn current_range(&self) -> Option<Range<usize>> {
        self.current.and_then(|i| self.matches.get(i).cloned())
    }

    /// Index of the first match that ends at or after `pos`.
    pub fn first_ending_at_or_after(&self, pos: usize) -> usize {
        self.matches.partition_point(|m| m.end < pos)
    }

    /// Moves the matches through applied edits (in the order they were
    /// applied); a match an edit touches inside is dropped.
    fn edited(&mut self, edits: &[AppliedEdit]) {
        if edits.len() > MAX_EDITS_TO_TRACK {
            *self = Self {
                current_background: self.current_background,
                background: self.background,
                ..Self::default()
            };
            return;
        }
        for e in edits {
            let delta = e.new_end as isize - e.old_end as isize;
            // Matches before the edit stay; matches after it move; matches
            // it touches inside are dropped.
            let before = self.matches.partition_point(|m| m.end <= e.start);
            let mut kept = Vec::with_capacity(self.matches.len());
            let mut current = None;
            let mut excluded = BTreeSet::new();
            for (i, m) in self.matches.drain(..).enumerate() {
                let moved = if i < before {
                    m
                } else if m.start >= e.old_end {
                    (m.start as isize + delta) as usize..(m.end as isize + delta) as usize
                } else {
                    continue;
                };
                if self.current == Some(i) {
                    current = Some(kept.len());
                }
                if self.excluded.contains(&i) {
                    excluded.insert(kept.len());
                }
                kept.push(moved);
            }
            self.matches = kept;
            self.current = current;
            self.excluded = excluded;
        }
    }
}

/// What happened in an editor, for its observers (tabs, status bar,
/// highlighting).
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// The text changed; replacements as applied.
    Edited(Vec<AppliedEdit>),
    /// Carets or selections moved.
    SelectionsChanged,
    /// Input the editor forwards instead of editing
    /// ([`Editor::set_forward_input`]).
    Input(ForwardedInput),
    /// A hyperlink (a char range of a line, see
    /// [`Highlighter::line_links`]) was clicked.
    LinkClicked { line: usize, range: Range<usize> },
    /// The mouse moved over the text without a button pressed, to another
    /// char or with other modifiers: the char offset under it (`None` when
    /// it left the text) and its window position.
    Hovered {
        offset: Option<usize>,
        position: gpui::Point<Pixels>,
        modifiers: Modifiers,
    },
    /// A click with the platform modifier (Cmd on macOS, Ctrl elsewhere,
    /// navigation) at a char offset; the caret did not move.
    ModifiedClick { offset: usize, modifiers: Modifiers },
    /// The whole text was replaced (reloaded from disk, or set by the
    /// editor's owner): positions from before are meaningless.
    Reloaded,
    /// The caret jumped as a navigation ([`Editor::jumped_from`]): `from`
    /// is where it was and `view_y` how far that line was from the top of
    /// the view. The navigation history records the place.
    Jumped { from: Point, view_y: Option<Pixels> },
    /// Undo (or redo) would revert (or apply again) a reload from disk and
    /// the owner asks first ([`Editor::set_confirm_reload_undo`]); nothing
    /// changed. [`Editor::undo_confirmed`] does it.
    ConfirmUndo { redo: bool },
    /// The mouse moved onto the line number of another line (`None`: off
    /// the line numbers). The owner may set a hover icon for it
    /// ([`Editor::set_gutter_hover_icon`]).
    GutterHovered { line: Option<usize> },
    /// A press on the line number of `line` (with
    /// [`Editor::set_line_number_clicks`]): any button, with modifiers
    /// (Alt-click, Shift-click, right-click).
    GutterClicked {
        line: usize,
        button: MouseButton,
        modifiers: Modifiers,
        position: gpui::Point<Pixels>,
    },
    /// A press on a decoration's change marker.
    ChangeMarkerClicked {
        id: DecorationId,
        position: gpui::Point<Pixels>,
    },
    /// A press on a decoration's clickable text after the end of a line.
    AfterLineTextClicked {
        id: DecorationId,
        position: gpui::Point<Pixels>,
    },
}

/// Input of an editor that forwards it (a console's standard input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardedInput {
    /// Typed or pasted text (line breaks as `\n`).
    Text(String),
    Enter,
    Tab,
    Backspace,
    Delete,
}

/// Clipboard metadata: the text is whole lines copied without a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct LineCopy {
    whole_lines: bool,
}

/// What a mouse drag selects.
#[derive(Debug, Clone)]
pub(crate) enum DragUnit {
    Char,
    Word(Range<usize>),
    Line(Range<usize>),
    /// Dragging the vertical scrollbar thumb; offset of the grab point.
    Scrollbar(Pixels),
}

/// A code editor for one buffer.
pub struct Editor {
    pub(crate) buffer: Buffer,
    pub(crate) selections: SelectionSet,
    focus: FocusHandle,
    config: EditorConfig,
    path: Option<PathBuf>,
    /// IME composition range (chars).
    pub(crate) marked: Option<Range<usize>>,
    /// Scroll offset of the text in pixels (both >= 0).
    pub(crate) scroll: gpui::Point<Pixels>,
    /// Bring the primary caret into view on the next frame.
    pub(crate) autoscroll: bool,
    /// Geometry of the last frame, for hit testing and the input method.
    pub(crate) layout: Option<LayoutInfo>,
    pub(crate) caret_visible: bool,
    pub(crate) drag: Option<DragUnit>,
    pub(crate) highlighter: Option<Box<dyn Highlighter>>,
    blink_epoch: usize,
    blink_task: Option<Task<()>>,
    /// The highlighter's background job in flight.
    highlight_task: Option<Task<()>>,
    word_stop: WordStop,
    /// Selections before each Extend Selection, for Shrink Selection.
    extend_stack: Vec<SelectionSet>,
    /// The selections the last Extend or Shrink Selection produced; any
    /// other change of the selections forgets the stack.
    extend_last: Option<SelectionSet>,
    /// Center the caret line on the next frame (Go to Line).
    pub(crate) center_caret: bool,
    /// Put the caret line this far from the top of the view on the next
    /// frame ([`Editor::go_to_place`]).
    pub(crate) caret_view_y: Option<Pixels>,
    /// A text field: no gutter, no scrollbar, no caret line (one line, or
    /// a few in search fields).
    pub(crate) single_line: bool,
    /// Text shown in an empty one-line field.
    pub(crate) placeholder: Option<SharedString>,
    /// The placeholder shows only while the field has no focus.
    pub(crate) placeholder_unfocused: bool,
    /// A one-line field shows its text in the error colour.
    pub(crate) error: bool,
    /// A one-line field in the UI font (search fields) instead of the
    /// editor font.
    pub(crate) ui_font: bool,
    /// A font of its own instead of the theme's editor font.
    pub(crate) font: Option<ui::theme::EditorFont>,
    /// No background of its own: the field's container paints it.
    pub(crate) transparent: bool,
    /// Search matches shown in the text.
    pub(crate) search: Option<SearchHighlights>,
    /// Bring this char offset into the middle of the view on the next
    /// frame when it is not visible.
    pub(crate) reveal: Option<usize>,
    /// A console: no line numbers, no caret row, no room below the last
    /// line, the console's colours.
    pub(crate) console: bool,
    /// Typing, Enter, Tab, Backspace, Delete and Paste are reported as
    /// [`EditorEvent::Input`] instead of editing the text.
    pub(crate) forward_input: bool,
    /// Scroll to the end of the text on the next frame.
    pub(crate) scroll_to_end: bool,
    /// Undoing or redoing a reload from disk asks the owner first.
    pub(crate) confirm_reload_undo: bool,
    /// The link under the last mouse press (line, char range, position):
    /// it is followed when the mouse is released at the same point.
    pub(crate) pressed_link: Option<(usize, Range<usize>, gpui::Point<Pixels>)>,
    /// Problems in the text, sorted by range, moved through edits.
    pub(crate) diagnostics: Marks,
    /// A char range drawn as a link (the symbol under the mouse with the
    /// navigation modifier).
    pub(crate) link_highlight: Option<Range<usize>>,
    /// How far the analysis of the text is (the inspection widget).
    pub(crate) analysis: Analysis,
    /// The mouse over the text without a button, as last reported in
    /// [`EditorEvent::Hovered`].
    pub(crate) hover: Option<Hover>,
    /// The problems whose error stripe marks are under the mouse (their
    /// tooltip).
    pub(crate) stripe_hover: Option<StripeHover>,
    /// A press on a problem's stripe mark (index, position): followed when
    /// released in place.
    pub(crate) pressed_stripe: Option<(usize, gpui::Point<Pixels>)>,
    /// Soft wraps, when on ([`Editor::set_soft_wraps`]).
    pub(crate) soft_wrap: Option<SoftWrap>,
    /// Carets (by head) at a soft wrap drawn at the end of the row before
    /// it rather than at the start of the next (after End, or moving up
    /// or down to the end of a row).
    pub(crate) lean_back: BTreeSet<usize>,
    /// What lines would wrap for in the last frame's width.
    pub(crate) last_wrap_params: Option<WrapParams>,
    /// The config's wrap rules, shared with the wrap parameters.
    pub(crate) wrap_rules: std::sync::Arc<[WrapRule]>,
    /// Continuation rows take the line's indentation plus this many
    /// columns; `None`: only the sign's column.
    pub(crate) soft_wrap_indent: Option<usize>,
    /// Signs at the soft wraps of every line, not only the caret's.
    pub(crate) show_all_soft_wraps: bool,
    /// A read-only view ([`Editor::set_viewer`]), and its hint when shown.
    pub(crate) viewer: bool,
    pub(crate) read_only_hint: Option<crate::viewer::ReadOnlyHint>,
    /// Gutter icons, change markers, line backgrounds, after-line texts and
    /// stripe marks of the editor's owners, moved through edits.
    pub(crate) decorations: Decorations,
    /// Presses on the line numbers are reported instead of moving the
    /// caret.
    pub(crate) line_number_clicks: bool,
    /// The line whose line number the mouse is over.
    pub(crate) gutter_hover: Option<usize>,
    /// The owner's hover icon and its line.
    pub(crate) gutter_hover_icon: Option<(usize, ui::icons::IconName)>,
    /// The decoration whose change marker the mouse is over (drawn wider).
    pub(crate) hovered_change: Option<DecorationId>,
    /// The decoration whose clickable after-line text the mouse is over
    /// (drawn as a link).
    pub(crate) hovered_after: Option<DecorationId>,
    _subscriptions: Vec<Subscription>,
}

/// The mouse over the text: the char under it and the modifiers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hover {
    pub offset: Option<usize>,
    pub position: gpui::Point<Pixels>,
    pub modifiers: Modifiers,
}

/// Problems under the mouse on the error stripe.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StripeHover {
    /// Indices of the marks, the nearest first.
    pub marks: Vec<usize>,
    /// Window y of the middle of the nearest mark (where the tooltip goes).
    pub y: Pixels,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Editor {
    pub fn new(buffer: Buffer, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |this, window, cx| {
                this.restart_blink(window, cx);
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.blink_task = None;
                this.buffer.break_undo_group();
                cx.notify();
            }),
        ];
        Self {
            buffer,
            selections: SelectionSet::caret(0),
            focus,
            config: EditorConfig::default(),
            path: None,
            marked: None,
            scroll: point(px(0.0), px(0.0)),
            autoscroll: false,
            layout: None,
            caret_visible: true,
            drag: None,
            highlighter: None,
            blink_epoch: 0,
            blink_task: None,
            highlight_task: None,
            word_stop: WordStop::platform(),
            extend_stack: Vec::new(),
            extend_last: None,
            center_caret: false,
            caret_view_y: None,
            single_line: false,
            placeholder: None,
            placeholder_unfocused: false,
            error: false,
            ui_font: false,
            font: None,
            transparent: false,
            search: None,
            reveal: None,
            console: false,
            forward_input: false,
            scroll_to_end: false,
            confirm_reload_undo: false,
            pressed_link: None,
            diagnostics: Marks::default(),
            link_highlight: None,
            analysis: Analysis::Off,
            hover: None,
            stripe_hover: None,
            pressed_stripe: None,
            soft_wrap: None,
            lean_back: BTreeSet::new(),
            last_wrap_params: None,
            wrap_rules: WrapRule::defaults().into(),
            soft_wrap_indent: Some(0),
            show_all_soft_wraps: false,
            viewer: false,
            read_only_hint: None,
            decorations: Decorations::default(),
            line_number_clicks: false,
            gutter_hover: None,
            gutter_hover_icon: None,
            hovered_change: None,
            hovered_after: None,
            _subscriptions: subscriptions,
        }
    }

    /// Makes this editor a console view, as the reference IDE sets up its
    /// console editor (`ConsoleViewUtil.setupConsoleEditor`): no line
    /// numbers, no caret row highlight, no extra page below the last line,
    /// the console's background and text colours.
    pub fn set_console(&mut self, console: bool) {
        self.console = console;
    }

    pub fn is_console(&self) -> bool {
        self.console
    }

    /// Reports typing, Enter, Tab, Backspace, Delete and Paste as
    /// [`EditorEvent::Input`] instead of editing (a console that sends
    /// them to a process).
    pub fn set_forward_input(&mut self, forward: bool) {
        self.forward_input = forward;
    }

    /// Undo and Redo of a reload from disk emit
    /// [`EditorEvent::ConfirmUndo`] instead of changing the text, as the
    /// reference asks "Undo Reload from Disk?" first.
    pub fn set_confirm_reload_undo(&mut self, confirm: bool) {
        self.confirm_reload_undo = confirm;
    }

    /// Undoes (or redoes) the last change without asking (after the
    /// owner's confirmation).
    pub fn undo_confirmed(&mut self, redo: bool, cx: &mut Context<Self>) {
        if !self.modification_allowed(cx) {
            return;
        }
        let done = if redo {
            self.buffer.redo()
        } else {
            self.buffer.undo()
        };
        if let Some((applied, selections)) = done {
            self.selections = selections;
            self.selections.clamp(self.buffer.len_chars());
            self.changed(applied, cx);
        }
    }

    /// Asks the owner first when the step is a reload from disk.
    fn undo_or_ask(&mut self, redo: bool, cx: &mut Context<Self>) {
        let kind = if redo {
            self.buffer.redo_kind()
        } else {
            self.buffer.undo_kind()
        };
        if self.confirm_reload_undo && kind == Some(text::EditKind::Reload) {
            cx.emit(EditorEvent::ConfirmUndo { redo });
        } else {
            self.undo_confirmed(redo, cx);
        }
    }

    /// Replaces text outside the undo history, also when the buffer is
    /// read-only (a console's output). Each edit is in the coordinates of
    /// the text left by the previous one; selections move with the text.
    pub fn replace_untracked(&mut self, edits: Vec<Edit>, cx: &mut Context<Self>) {
        if edits.is_empty() {
            return;
        }
        let mut applied = Vec::with_capacity(edits.len());
        for edit in edits {
            let a = self.buffer.replace_untracked(edit.range, &edit.text);
            let map = |pos: usize| -> usize {
                if pos <= a.start {
                    pos
                } else if pos >= a.old_end {
                    pos - a.old_end + a.new_end
                } else {
                    a.start
                }
            };
            self.selections.update(|s| {
                s.anchor = map(s.anchor);
                s.head = map(s.head);
            });
            applied.push(a);
        }
        self.selections.clamp(self.buffer.len_chars());
        self.marked = None;
        if let Some(h) = &mut self.highlighter {
            h.edited(&self.buffer, &applied);
        }
        if let Some(search) = &mut self.search {
            search.edited(&applied);
        }
        self.text_edited(&applied);
        self.wrap_edited(&applied, cx);
        self.schedule_highlighting(cx);
        cx.emit(EditorEvent::Edited(applied));
        cx.notify();
    }

    /// Reports input instead of editing when the editor forwards it.
    fn forwarded(
        &mut self,
        input: impl FnOnce() -> ForwardedInput,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.forward_input {
            return false;
        }
        cx.emit(EditorEvent::Input(input()));
        true
    }

    /// Scrolls to the end of the text on the next frame (the last line at
    /// the bottom of the view).
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_end = true;
        cx.notify();
    }

    /// The view shows the end of the text (or will on the next frame).
    pub fn is_scrolled_to_end(&self) -> bool {
        if self.scroll_to_end {
            return true;
        }
        let Some(layout) = &self.layout else {
            return true;
        };
        self.scroll.y + px(0.5) >= self.max_scroll_y_for(layout.bounds.size.height)
    }

    /// Makes this editor a one-line text field (dialogs, search fields).
    pub fn set_single_line(&mut self, single_line: bool) {
        self.single_line = single_line;
    }

    /// Shows the text of a one-line field in the UI font instead of the
    /// editor font (search fields, as in the reference IDE).
    pub fn set_ui_font(&mut self, ui_font: bool) {
        self.ui_font = ui_font;
    }

    /// Shows the text in `font` instead of the theme's editor font (the
    /// preview of Editor > Font, before the font is applied); `None` goes
    /// back to the theme's.
    pub fn set_font(&mut self, font: Option<ui::theme::EditorFont>, cx: &mut Context<Self>) {
        if self.font != font {
            self.font = font;
            cx.notify();
        }
    }

    /// Leaves the background to the field's container.
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }

    /// Text shown in the empty one-line field ("Search") or console-style
    /// text area ("Commit Message").
    pub fn set_placeholder(&mut self, placeholder: Option<SharedString>) {
        self.placeholder = placeholder;
    }

    /// Shows the placeholder only while the field has no focus, as the
    /// reference IDE's dialog fields do (an empty field with the focus
    /// shows just the caret).
    pub fn set_placeholder_unfocused(&mut self, unfocused: bool) {
        self.placeholder_unfocused = unfocused;
    }

    /// The placeholder the field shows now: an empty one-line field or
    /// console-style text area shows it, unless it has the focus and shows
    /// it only without.
    pub fn shown_placeholder(&self, focused: bool) -> Option<&SharedString> {
        self.placeholder.as_ref().filter(|_| {
            (self.single_line || self.console)
                && self.buffer.len_chars() == 0
                && !(self.placeholder_unfocused && focused)
        })
    }

    /// Shows the text of a one-line field in the theme's error colour (a
    /// search that finds nothing).
    pub fn set_error(&mut self, error: bool, cx: &mut Context<Self>) {
        if self.error != error {
            self.error = error;
            cx.notify();
        }
    }

    pub fn is_error(&self) -> bool {
        self.error
    }

    /// Replaces the whole text without an undo step and selects it (a
    /// field filled in by its owner).
    pub fn set_text_selected(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.buffer.text() != text {
            self.buffer = Buffer::new(text);
            self.buffer_reloaded(cx);
        }
        let len = self.buffer.len_chars();
        self.set_selections(SelectionSet::single(Selection::new(0, len)), cx);
    }

    // ---- search highlights ------------------------------------------------

    /// The search matches shown in the text.
    pub fn search_highlights(&self) -> Option<&SearchHighlights> {
        self.search.as_ref()
    }

    /// Shows search matches in the text, or removes them.
    pub fn set_search_highlights(
        &mut self,
        highlights: Option<SearchHighlights>,
        cx: &mut Context<Self>,
    ) {
        if self.search != highlights {
            self.search = highlights;
            cx.notify();
        }
    }

    /// Makes another match the current one.
    pub fn set_search_current(&mut self, current: Option<usize>, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.search
            && h.current != current
        {
            h.current = current;
            cx.notify();
        }
    }

    /// Leaves a match out of Replace All (struck through), or takes it back.
    pub fn set_search_excluded(&mut self, index: usize, excluded: bool, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.search {
            let changed = if excluded {
                h.excluded.insert(index)
            } else {
                h.excluded.remove(&index)
            };
            if changed {
                cx.notify();
            }
        }
    }

    // ---- diagnostics and links -----------------------------------------------

    /// Replaces the problems shown in the text (char ranges in the current
    /// text, clamped to it; sorted here by range). An empty range at a line
    /// end is drawn after the end of the line; any other empty range covers
    /// the char after it. The marks then move with the text (typing, undo)
    /// until they are replaced.
    pub fn set_diagnostics(&mut self, marks: Vec<DiagnosticMark>, cx: &mut Context<Self>) {
        if self.diagnostics.set(marks, &self.buffer) {
            self.stripe_hover = None;
            self.pressed_stripe = None;
            cx.notify();
        }
    }

    /// The problems shown in the text, sorted by range, in the current text.
    pub fn diagnostics(&self) -> &[DiagnosticMark] {
        self.diagnostics.as_slice()
    }

    /// The problems at a char offset (the char under the mouse, the caret):
    /// those over the char and the empty ones there (drawn after the end
    /// of their line), the most severe first. The data of the hover popup.
    pub fn diagnostics_at(&self, offset: usize) -> Vec<&DiagnosticMark> {
        self.diagnostics
            .at(offset)
            .into_iter()
            .map(|i| self.diagnostics.get(i))
            .collect()
    }

    /// The problems by level as the inspection widget counts them (every
    /// message by its own level).
    pub fn problem_counts(&self) -> ProblemCounts {
        self.diagnostics.counts()
    }

    /// Tells the editor how far the analysis of its text is: the inspection
    /// widget shows from [`Analysis::Running`] on, translucent until
    /// [`Analysis::Done`].
    pub fn set_analysis(&mut self, analysis: Analysis, cx: &mut Context<Self>) {
        if self.analysis != analysis {
            self.analysis = analysis;
            cx.notify();
        }
    }

    pub fn analysis(&self) -> Analysis {
        self.analysis
    }

    /// Moves the caret to the next (`forward`) or previous problem, as the
    /// reference's Next / Previous Highlighted Error: errors first, wrapping
    /// around; information is skipped. Returns whether there was one.
    pub fn go_to_problem(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let caret = self.selections.primary().head;
        match problem_to_go(self.diagnostics(), Some(caret), forward) {
            Some(i) => {
                self.show_problem(i, cx);
                self.jumped_from(caret, cx);
                true
            }
            None => false,
        }
    }

    /// Moves the caret to the first problem (errors first): the inspection
    /// widget's click until the Problems tool window exists.
    pub fn go_to_first_problem(&mut self, cx: &mut Context<Self>) -> bool {
        match problem_to_go(self.diagnostics(), None, true) {
            Some(i) => {
                self.show_problem(i, cx);
                true
            }
            None => false,
        }
    }

    /// The caret at the start of a problem (no selection, one caret), its
    /// line brought into the middle of the view when hidden.
    fn show_problem(&mut self, index: usize, cx: &mut Context<Self>) {
        let pos = self.diagnostics.get(index).range.start;
        self.set_selections(SelectionSet::caret(pos), cx);
        self.reveal(pos, cx);
    }

    /// A problem's error stripe mark was clicked: the caret goes to the
    /// problem, its line to the middle of the view, the focus to the
    /// editor.
    pub(crate) fn stripe_clicked(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        if index < self.diagnostics().len() {
            let pos = self.diagnostics.get(index).range.start;
            self.set_selections(SelectionSet::caret(pos), cx);
            self.center_caret = true;
        }
    }

    // ---- hover --------------------------------------------------------------

    /// The mouse moved without a button: reports the char under it when it
    /// or the modifiers change ([`EditorEvent::Hovered`], `None` when it
    /// leaves the text), and finds the stripe marks under it.
    pub(crate) fn mouse_moved(
        &mut self,
        position: gpui::Point<Pixels>,
        modifiers: Modifiers,
        over_text: bool,
        over_editor: bool,
        cx: &mut Context<Self>,
    ) {
        self.decoration_hover(position, over_editor, cx);
        let stripe = self.layout.as_ref().filter(|_| over_editor).and_then(|l| {
            if !l.stripe?.contains(&position) {
                return None;
            }
            let marks = l.stripe_marks_near(position.y);
            let y = l.stripe_mark_middle(*marks.first()?)?;
            Some(StripeHover { marks, y })
        });
        if self.stripe_hover != stripe {
            self.stripe_hover = stripe;
            cx.notify();
        }
        let offset = if over_text {
            self.char_at(position)
        } else {
            None
        };
        let changed = match self.hover {
            None => offset.is_some(),
            Some(last) => {
                last.offset != offset
                    || (last.modifiers != modifiers && (last.offset.is_some() || offset.is_some()))
            }
        };
        self.hover = over_text.then_some(Hover {
            offset,
            position,
            modifiers,
        });
        if changed {
            cx.emit(EditorEvent::Hovered {
                offset,
                position,
                modifiers,
            });
        }
    }

    /// Modifiers changed while the editor has the focus: reported while the
    /// mouse is over a char.
    pub(crate) fn modifiers_changed(&mut self, modifiers: Modifiers, cx: &mut Context<Self>) {
        let Some(hover) = &mut self.hover else {
            return;
        };
        if hover.modifiers == modifiers {
            return;
        }
        hover.modifiers = modifiers;
        if let Some(offset) = hover.offset {
            cx.emit(EditorEvent::Hovered {
                offset: Some(offset),
                position: hover.position,
                modifiers,
            });
        }
    }

    /// The char under the mouse as last reported (`None` when the mouse is
    /// not over a char).
    pub fn hovered(&self) -> Option<usize> {
        self.hover.and_then(|h| h.offset)
    }

    /// Draws a char range as a link with the hand pointer (the symbol
    /// under the mouse while the navigation modifier is held), or removes
    /// it.
    pub fn set_link_highlight(&mut self, range: Option<Range<usize>>, cx: &mut Context<Self>) {
        if self.link_highlight != range {
            self.link_highlight = range;
            cx.notify();
        }
    }

    pub fn link_highlight(&self) -> Option<Range<usize>> {
        self.link_highlight.clone()
    }

    // ---- geometry ----------------------------------------------------------

    /// The char offset nearest to a window position over the text, from
    /// the last frame; `None` before the first frame and outside the text
    /// area (gutter, scrollbar, outside the editor).
    pub fn offset_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        layout
            .over_text(position)
            .then(|| self.offset_for_position(position))
    }

    /// Window bounds of the text area in the last frame (without the
    /// gutter and the error stripe), for placing popups.
    pub fn text_area_bounds(&self) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let right = layout.stripe.map_or(layout.bounds.right(), |s| s.left());
        Some(Bounds::from_corners(
            point(layout.text_x, layout.bounds.top()),
            point(right, layout.bounds.bottom()),
        ))
    }

    /// Window bounds of a char range in the last frame: from its start to
    /// its end on one line; for a range over several lines, the box around
    /// both ends. An empty range has the width of a char (where popups
    /// about a position go). `None` before the first frame.
    pub fn bounds_for_range(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        let start = self.bounds_for_offset(range.start)?;
        if range.end <= range.start {
            return Some(start);
        }
        let end = self.bounds_for_offset(range.end)?;
        if end.top() == start.top() {
            return Some(Bounds::from_corners(
                start.origin,
                point(end.left().max(start.left()), start.bottom()),
            ));
        }
        Some(Bounds::from_corners(
            point(start.left().min(end.left()), start.top()),
            point(start.right().max(end.left()), end.bottom()),
        ))
    }

    /// Scrolls `pos` into the middle of the view on the next frame unless
    /// it is visible already.
    pub fn reveal(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.reveal = Some(pos.min(self.buffer.len_chars()));
        cx.notify();
    }

    /// Selects `range` with the caret at its end and reveals its start (a
    /// search match).
    pub fn select_and_reveal(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.set_selections(
            SelectionSet::single(Selection::new(range.start, range.end)),
            cx,
        );
        self.reveal(range.start, cx);
    }

    // ---- state ------------------------------------------------------------

    pub(crate) fn focus_handle_ref(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Mutable access for owners that load or save (no undo step).
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    pub fn config(&self) -> &EditorConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: EditorConfig) {
        if config.wrap_rules[..] != self.wrap_rules[..] {
            self.wrap_rules = config.wrap_rules.clone().into();
        }
        self.config = config;
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn set_word_stop(&mut self, stop: WordStop) {
        self.word_stop = stop;
    }

    pub fn set_highlighter(
        &mut self,
        highlighter: Option<Box<dyn Highlighter>>,
        cx: &mut Context<Self>,
    ) {
        self.highlighter = highlighter;
        self.highlight_task = None;
        if let Some(h) = &mut self.highlighter {
            h.reset(&self.buffer);
        }
        self.schedule_highlighting(cx);
        cx.notify();
    }

    /// Highlight spans of one line as the highlighter has them now (empty
    /// while the first parse runs).
    pub fn highlights(&mut self, line: usize) -> Vec<crate::highlight::Span> {
        let mut out = Vec::new();
        if line < self.buffer.len_lines()
            && let Some(h) = &mut self.highlighter
        {
            h.prepare(&self.buffer, line..line + 1);
            h.line_spans(line, &mut out);
        }
        out
    }

    /// Highlighting reflects the current text.
    pub fn highlighting_current(&self) -> bool {
        self.highlighter.as_ref().is_none_or(|h| h.is_current())
    }

    /// Runs the highlighter's next job on the background executor; the
    /// result is applied on the UI thread and the next job follows.
    fn schedule_highlighting(&mut self, cx: &mut Context<Self>) {
        if self.highlight_task.is_some() {
            return;
        }
        let Some(job) = self
            .highlighter
            .as_mut()
            .and_then(|h| h.next_job(&self.buffer))
        else {
            return;
        };
        let work = cx.background_spawn(async move { job() });
        self.highlight_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.highlight_task = None;
                if let Some(h) = &mut this.highlighter {
                    h.job_done(result, &this.buffer);
                }
                this.schedule_highlighting(cx);
                cx.notify();
            });
        }));
    }

    pub fn highlighter(&self) -> Option<&dyn Highlighter> {
        self.highlighter.as_deref()
    }

    pub fn highlighter_mut(&mut self) -> Option<&mut (dyn Highlighter + 'static)> {
        self.highlighter.as_deref_mut()
    }

    /// Zero-based position of the primary caret.
    pub fn caret_point(&self) -> Point {
        self.buffer.offset_to_point(self.selections.primary().head)
    }

    /// Selected chars and line breaks of the primary selection.
    pub fn selection_stats(&self) -> (usize, usize) {
        let range = self.selections.primary().range();
        if range.is_empty() {
            return (0, 0);
        }
        let breaks = self.buffer.char_to_line(range.end) - self.buffer.char_to_line(range.start);
        (range.len(), breaks)
    }

    /// Vertical scroll offset in pixels.
    pub fn scroll_top(&self) -> Pixels {
        self.scroll.y
    }

    /// Horizontal scroll offset in pixels.
    pub fn scroll_left(&self) -> Pixels {
        self.scroll.x
    }

    /// First and last line drawn in the last frame.
    pub fn visible_lines(&self) -> Option<Range<usize>> {
        self.layout.as_ref().map(|l| l.visible.clone())
    }

    /// Geometry of the last frame: window position of line 0, column 0
    /// (scrolling applied), line height and the width of a character.
    pub fn text_geometry(&self) -> Option<(gpui::Point<Pixels>, Pixels, Pixels)> {
        let l = self.layout.as_ref()?;
        Some((
            point(l.text_x - self.scroll.x, l.bounds.top() - self.scroll.y),
            l.line_height,
            l.char_width,
        ))
    }

    /// Window bounds of the primary caret's character in the last frame
    /// (where hints about the caret go).
    pub fn caret_bounds(&self) -> Option<gpui::Bounds<Pixels>> {
        let head = self.selections.primary().head;
        self.bounds_for_offset_lean(head, self.leans_back(head))
    }

    /// Lines shaped in the last frame (virtualization check).
    pub fn shaped_line_count(&self) -> usize {
        self.layout.as_ref().map_or(0, |l| l.lines.len())
    }

    /// The editor's state as JSON (`editor.state`).
    pub fn state_json(&self) -> serde_json::Value {
        let point = |p: usize| {
            let p = self.buffer.offset_to_point(p);
            json!({ "line": p.line, "column": p.column })
        };
        let (chars, breaks) = self.selection_stats();
        let counts = self.problem_counts();
        json!({
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "lines": self.buffer.len_lines(),
            "chars": self.buffer.len_chars(),
            "version": self.buffer.version(),
            "modified": self.buffer.is_modified(),
            "read_only": self.buffer.is_read_only(),
            "line_ending": self.buffer.line_ending().label(),
            "encoding": self.buffer.encoding().label(),
            "caret": point(self.selections.primary().head),
            "selected_chars": chars,
            "selected_line_breaks": breaks,
            "selections": self.selections.iter().map(|s| json!({
                "anchor": point(s.anchor),
                "head": point(s.head),
            })).collect::<Vec<_>>(),
            "can_undo": self.buffer.can_undo(),
            "can_redo": self.buffer.can_redo(),
            "visible_lines": self.visible_lines().map(|r| json!([r.start, r.end])),
            "diagnostics": self.diagnostics().iter().map(|m| json!({
                "range": { "start": point(m.range.start), "end": point(m.range.end) },
                "severity": m.severity.name(),
                "unnecessary": m.unnecessary,
                "deprecated": m.deprecated,
                "messages": m.messages.iter().map(|msg| json!({
                    "severity": msg.severity.name(),
                    "text": msg.text,
                    "source": msg.source,
                    "code": msg.code,
                    "href": msg.href,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "link_highlight": self.link_highlight.as_ref().map(|r| json!({
                "start": point(r.start),
                "end": point(r.end),
            })),
            "analysis": self.analysis.name(),
            "soft_wraps": self.soft_wraps(),
            "wrap_columns": self.wrap_columns(),
            "rows": self.display_rows(),
            "caret_row": self.primary_row(),
            "wrap_signs": self.layout.as_ref().map(|l| l.wrap_signs),
            "problems": {
                "errors": counts.errors,
                "warnings": counts.warnings,
                "weak_warnings": counts.weak_warnings,
            },
            "hovered": self.hovered().map(point),
            "decorations": self.decorations_json(),
            "gutter_hover": self.gutter_hover,
        })
    }

    // ---- changes --------------------------------------------------------

    /// Applies edits as one undo step and moves the carets.
    pub fn edit(
        &mut self,
        edits: Vec<Edit>,
        kind: EditKind,
        carets: CaretPlacement,
        cx: &mut Context<Self>,
    ) {
        if !edits.is_empty() && !self.modification_allowed(cx) {
            return;
        }
        let mut selections = self.selections.clone();
        let applied = self.buffer.transact(&mut selections, edits, kind, carets);
        if applied.is_empty() {
            return;
        }
        self.selections = selections;
        self.changed(applied, cx);
    }

    fn changed(&mut self, applied: Vec<AppliedEdit>, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.highlighter {
            h.edited(&self.buffer, &applied);
        }
        if let Some(search) = &mut self.search {
            search.edited(&applied);
        }
        self.text_edited(&applied);
        self.wrap_edited(&applied, cx);
        self.schedule_highlighting(cx);
        self.autoscroll = true;
        self.caret_visible = true;
        self.blink_epoch += 1;
        cx.emit(EditorEvent::Edited(applied));
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    /// Replaces the selections; the caret is shown and scrolled into view.
    pub fn set_selections(&mut self, selections: SelectionSet, cx: &mut Context<Self>) {
        let mut selections = selections;
        selections.clamp(self.buffer.len_chars());
        if selections != self.selections {
            self.buffer.break_undo_group();
        }
        self.selections = selections;
        self.marked = None;
        self.lean_back.clear();
        self.hide_read_only_hint(cx);
        self.autoscroll = true;
        self.caret_visible = true;
        self.blink_epoch += 1;
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    /// Inserts `text` at every caret, replacing the selections.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let kind = if text.contains('\n') {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), text))
            .collect();
        self.edit(edits, kind, CaretPlacement::AfterEachEdit, cx);
    }

    /// Moves what is anchored in the text through applied edits: the
    /// problems move with it; the link highlight was about the old text.
    fn text_edited(&mut self, applied: &[AppliedEdit]) {
        self.diagnostics.edited(applied);
        self.decorations.edited(applied);
        self.decoration_hover_reset();
        self.link_highlight = None;
        // Indices of marks may have changed.
        self.stripe_hover = None;
        self.pressed_stripe = None;
    }

    /// Brings the text to the file's on disk as `plan` (made off the UI
    /// thread from this editor's text) works it out: one undo step, the
    /// buffer unmodified and read-only as given, carets as
    /// [`text::Buffer::apply_reload`] places them, the view not scrolled.
    /// Returns false, changing nothing, when the text changed since the
    /// plan was made.
    pub fn reload(
        &mut self,
        plan: text::ReloadPlan,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut selections = self.selections.clone();
        let changed = match self.buffer.apply_reload(plan, &mut selections) {
            Ok(applied) => {
                // Decorations follow the replacement, then stay at their
                // lines in `buffer_reloaded`.
                if let Some(a) = &applied {
                    self.decorations.edited(std::slice::from_ref(a));
                    let _ = self.decorations.lines(&self.buffer);
                }
                applied.is_some()
            }
            Err(text::StalePlan) => return false,
        };
        self.buffer.set_read_only(read_only);
        if changed {
            self.selections = selections;
            self.marked = None;
            self.buffer_reloaded(cx);
            let y = self.scroll.y;
            self.set_scroll_y(y, cx);
        }
        cx.notify();
        true
    }

    /// Notifies observers that the buffer was replaced from disk or saved.
    pub fn buffer_reloaded(&mut self, cx: &mut Context<Self>) {
        self.selections.clamp(self.buffer.len_chars());
        // Positions in the old text mean nothing in the new one; the
        // decorations stay at their lines.
        self.diagnostics.clear();
        self.decorations.text_replaced(&self.buffer);
        self.decoration_hover_reset();
        self.link_highlight = None;
        self.stripe_hover = None;
        self.pressed_stripe = None;
        if let Some(h) = &mut self.highlighter {
            h.reset(&self.buffer);
        }
        self.highlight_task = None;
        self.wrap_reloaded(cx);
        self.schedule_highlighting(cx);
        cx.emit(EditorEvent::Reloaded);
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    // ---- caret blinking ------------------------------------------------------

    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.blink_epoch += 1;
        self.blink_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut seen = None;
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    // Typing or moving shows the caret for a full interval.
                    if seen == Some(this.blink_epoch) {
                        this.caret_visible = !this.caret_visible;
                    } else {
                        this.caret_visible = true;
                    }
                    seen = Some(this.blink_epoch);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    // ---- movement -------------------------------------------------------------

    fn move_carets(
        &mut self,
        extend: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&Self, &Selection) -> usize,
    ) {
        let mut set = self.selections.clone();
        let targets: Vec<usize> = set.iter().map(|s| f(self, s)).collect();
        let mut i = 0;
        set.update(|s| {
            let to = targets[i];
            i += 1;
            if extend {
                s.extend_to(to);
            } else {
                s.collapse_to(to);
            }
        });
        self.set_selections(set, cx);
    }

    /// Moves the carets by visual rows (lines without soft wraps), each
    /// keeping its goal column.
    fn vertical(&mut self, rows: isize, extend: bool, cx: &mut Context<Self>) {
        let mut set = self.selections.clone();
        let last = self.display_rows() - 1;
        let mut leaning = BTreeSet::new();
        set.update(|s| {
            let head = if !extend && !s.is_empty() {
                if rows < 0 { s.start() } else { s.end() }
            } else {
                s.head
            };
            let lean = head == s.head && self.leans_back(head);
            let (row, col) = self.visual_point(head, lean);
            let goal = s.goal.unwrap_or(col);
            let target = (row as isize + rows).clamp(0, last as isize) as usize;
            let (to, to_lean) = if target == row {
                // Up on the first row or down on the last: text start or end.
                if rows < 0 && row == 0 {
                    (0, false)
                } else if rows > 0 && row == last {
                    (self.buffer.len_chars(), false)
                } else {
                    (head, lean)
                }
            } else {
                self.offset_at_visual(target, goal)
            };
            if extend {
                s.head = to;
            } else {
                s.anchor = to;
                s.head = to;
            }
            s.goal = Some(goal);
            if to_lean {
                leaning.insert(to);
            }
        });
        self.set_selections(set, cx);
        self.lean_back = leaning;
    }

    fn page_lines(&self) -> isize {
        self.layout
            .as_ref()
            .map_or(30, |l| l.viewport_lines.saturating_sub(1).max(1)) as isize
    }

    fn page(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let n = self.page_lines();
        let line_height = self.layout.as_ref().map_or(px(20.0), |l| l.line_height);
        let delta = line_height * n as f32;
        self.scroll.y = if down {
            self.scroll.y + delta
        } else {
            (self.scroll.y - delta).max(px(0.0))
        };
        self.vertical(if down { n } else { -n }, extend, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            if s.is_empty() {
                movement::prev_grapheme(&e.buffer, s.head)
            } else {
                s.start()
            }
        });
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            if s.is_empty() {
                movement::next_grapheme(&e.buffer, s.head)
            } else {
                s.end()
            }
        });
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| movement::word_left(&e.buffer, s.head));
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        self.move_carets(false, cx, move |e, s| {
            movement::word_right(&e.buffer, s.head, stop)
        });
    }

    fn move_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| e.row_start_target(s.head));
    }

    fn move_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to_row_ends(false, cx);
    }

    /// End: every caret to the end of its visual row, or from there to the
    /// end of its line.
    fn move_to_row_ends(&mut self, extend: bool, cx: &mut Context<Self>) {
        let targets: Vec<(usize, bool)> = self
            .selections
            .iter()
            .map(|s| self.row_end_target(s.head))
            .collect();
        let mut i = 0;
        let mut set = self.selections.clone();
        set.update(|s| {
            let to = targets[i].0;
            i += 1;
            if extend {
                s.extend_to(to);
            } else {
                s.collapse_to(to);
            }
        });
        self.set_selections(set, cx);
        self.lean_back = targets.iter().filter(|t| t.1).map(|t| t.0).collect();
    }

    fn move_document_start(
        &mut self,
        _: &MoveDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.selections.primary().head;
        self.set_selections(SelectionSet::caret(0), cx);
        self.jumped_from(from, cx);
    }

    fn move_document_end(&mut self, _: &MoveDocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        let from = self.selections.primary().head;
        self.set_selections(SelectionSet::caret(self.buffer.len_chars()), cx);
        self.jumped_from(from, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, false, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::prev_grapheme(&e.buffer, s.head));
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::next_grapheme(&e.buffer, s.head));
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::word_left(&e.buffer, s.head));
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        self.move_carets(true, cx, move |e, s| {
            movement::word_right(&e.buffer, s.head, stop)
        });
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| e.row_start_target(s.head));
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to_row_ends(true, cx);
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_carets(true, cx, |_, _| 0);
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_carets(true, cx, |e, _| e.buffer.len_chars());
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, true, cx);
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.buffer.len_chars();
        self.set_selections(SelectionSet::single(Selection::new(0, len)), cx);
    }

    /// Extend Selection: the word at the caret, then the enclosing syntax
    /// nodes (when the file type has a syntax tree), then the text of the
    /// lines, the whole lines and the whole file; always the smallest range
    /// that is larger than the selection.
    fn select_word(&mut self, _: &SelectWord, _: &mut Window, cx: &mut Context<Self>) {
        if self.extend_last.as_ref() != Some(&self.selections) {
            self.extend_stack.clear();
        }
        let s = *self.selections.primary();
        let range = s.range();
        let mut candidates = vec![movement::word_range(&self.buffer, s.head)];
        if let Some(h) = &self.highlighter {
            candidates.extend(h.enclosing_ranges(&self.buffer, range.clone()));
        }
        let lines = self.selected_lines(&s);
        let first_start = self.buffer.line_start(lines.start);
        let last_end = self.buffer.line_end(lines.end - 1);
        let first_text = self.buffer.line_text(lines.start);
        let last_text = self.buffer.line_text(lines.end - 1);
        let content_start = first_start + leading_ws(&first_text);
        let content_end =
            last_end - (last_text.chars().count() - last_text.trim_end().chars().count());
        candidates.push(content_start..content_end.max(content_start));
        candidates.push(first_start..last_end);
        let after_last = if lines.end < self.buffer.len_lines() {
            self.buffer.line_start(lines.end)
        } else {
            self.buffer.len_chars()
        };
        candidates.push(first_start..after_last);
        candidates.push(0..self.buffer.len_chars());
        let Some(next) = candidates
            .into_iter()
            .filter(|c| c.start <= range.start && range.end <= c.end && c.len() > range.len())
            .min_by_key(|c| c.len())
        else {
            return;
        };
        self.extend_stack.push(self.selections.clone());
        self.set_selections(
            SelectionSet::single(Selection::new(next.start, next.end)),
            cx,
        );
        self.extend_last = Some(self.selections.clone());
    }

    fn shrink_selection(&mut self, _: &ShrinkSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.extend_last.as_ref() != Some(&self.selections) {
            self.extend_stack.clear();
            return;
        }
        if let Some(previous) = self.extend_stack.pop() {
            self.set_selections(previous, cx);
            self.extend_last = Some(self.selections.clone());
        }
    }

    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.selections.len() == 1 && self.selections.primary().is_empty() {
            cx.propagate();
            return;
        }
        let head = self.selections.primary().head;
        self.set_selections(SelectionSet::caret(head), cx);
    }

    fn go_to_line(&mut self, a: &GoToLine, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to(a.line, a.column, cx);
    }

    fn next_problem(&mut self, _: &NextProblem, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_problem(true, cx);
    }

    fn previous_problem(&mut self, _: &PreviousProblem, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_problem(false, cx);
    }

    /// Moves the caret to a one-based line and column (both clamped) and
    /// centers the line in the view.
    pub fn go_to(&mut self, line: usize, column: Option<usize>, cx: &mut Context<Self>) {
        let line = (line.max(1) - 1).min(self.buffer.len_lines() - 1);
        let column = column.unwrap_or(1).max(1) - 1;
        let pos = self.buffer.line_start(line) + column.min(self.buffer.line_len(line));
        self.set_selections(SelectionSet::caret(pos), cx);
        self.center_caret = true;
    }

    fn set_selection(&mut self, a: &SetSelection, _: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.buffer.point_to_offset(a.anchor.into());
        let head = self.buffer.point_to_offset(a.head.into());
        self.set_selections(SelectionSet::single(Selection::new(anchor, head)), cx);
    }

    fn scroll_to_line(&mut self, a: &ScrollToLine, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.row_height();
        let line = a.line.min(self.buffer.len_lines().saturating_sub(1));
        self.scroll.y = line_height * self.line_row(line) as f32;
        cx.notify();
    }

    // ---- editing --------------------------------------------------------------

    fn type_text(&mut self, a: &Type, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Text(a.text.clone()), cx) {
            return;
        }
        self.type_str(&a.text, cx);
    }

    /// Typed text: a single character goes through bracket and quote
    /// pairing, anything else is inserted as it is.
    pub(crate) fn type_str(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if self.marked.is_none() => self.type_char(c, cx),
            _ => self.insert(text, cx),
        }
    }

    /// The caret is between an opening bracket or quote and its closing one.
    fn is_empty_pair_at(&self, pos: usize) -> bool {
        let (Some(prev), Some(next)) = (
            pos.checked_sub(1).and_then(|p| self.buffer.char_at(p)),
            self.buffer.char_at(pos),
        ) else {
            return false;
        };
        self.config.brackets.contains(&(prev, next))
            || (prev == next && self.config.quotes.contains(&prev))
    }

    fn closing_bracket(&self, c: char) -> Option<char> {
        self.config.brackets.iter().find(|b| b.0 == c).map(|b| b.1)
    }

    /// Types one character at the caret: wraps a selection in brackets or
    /// quotes, types over a closing bracket or quote, closes an opening
    /// one, and turns a third quote into a triple-quoted pair.
    fn type_char(&mut self, c: char, cx: &mut Context<Self>) {
        let is_quote = self.config.quotes.contains(&c);
        let closing = self.closing_bracket(c).or(is_quote.then_some(c));
        let is_closer = is_quote || self.config.brackets.iter().any(|b| b.1 == c);
        if self.selections.len() != 1 || (closing.is_none() && !is_closer) {
            self.insert(&c.to_string(), cx);
            return;
        }
        let s = *self.selections.primary();
        if !s.is_empty() {
            match closing {
                Some(close) => {
                    let text = format!("{c}{}{close}", self.buffer.slice(s.range()));
                    let after = Selection::new(s.anchor + 1, s.head + 1);
                    self.edit(
                        vec![Edit::new(s.range(), text)],
                        EditKind::Other,
                        CaretPlacement::Explicit(SelectionSet::single(after)),
                        cx,
                    );
                }
                None => self.insert(&c.to_string(), cx),
            }
            return;
        }
        let pos = s.head;
        let next = self.buffer.char_at(pos);
        let prev = pos.checked_sub(1).and_then(|p| self.buffer.char_at(p));
        let line = self.buffer.char_to_line(pos);
        let before = self.buffer.slice(self.buffer.line_start(line)..pos);
        // A third quote right after an empty pair: a triple-quoted string.
        if is_quote && self.config.triple_quotes {
            let tail: Vec<char> = before.chars().rev().take(3).collect();
            if tail.len() >= 2 && tail[0] == c && tail[1] == c && tail.get(2) != Some(&c) {
                let text: String = std::iter::repeat_n(c, 4).collect();
                let caret = pos + 1;
                self.edit(
                    vec![Edit::insert(pos, text)],
                    EditKind::Typing,
                    CaretPlacement::Explicit(SelectionSet::caret(caret)),
                    cx,
                );
                return;
            }
        }
        // Inside a string of this quote: an odd count of it before the caret.
        let inside = is_quote && before.chars().filter(|q| *q == c).count() % 2 == 1;
        // Type over the closing bracket or quote.
        if is_closer && next == Some(c) && (!is_quote || inside) {
            self.buffer.break_undo_group();
            self.set_selections(SelectionSet::caret(pos + 1), cx);
            return;
        }
        let Some(close) = closing else {
            self.insert(&c.to_string(), cx);
            return;
        };
        let next_allows = next.is_none_or(|n| {
            n.is_whitespace()
                || self.config.brackets.iter().any(|b| b.1 == n)
                || matches!(n, ',' | ';' | ':')
        });
        let quote_allowed = !is_quote || {
            // Not inside a string already and not right after a word, unless
            // the word is a string prefix.
            let word: String = before
                .chars()
                .rev()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            let prefix_ok = word.is_empty()
                || (word.chars().count() <= 2
                    && word
                        .chars()
                        .all(|ch| self.config.string_prefixes.contains(&ch)));
            !inside && prefix_ok && prev != Some(c)
        };
        if next_allows && quote_allowed {
            self.edit(
                vec![Edit::insert(pos, format!("{c}{close}"))],
                EditKind::Typing,
                CaretPlacement::Explicit(SelectionSet::caret(pos + 1)),
                cx,
            );
        } else {
            self.insert(&c.to_string(), cx);
        }
    }

    /// Leading whitespace of the line of `pos`, up to `pos`.
    fn indent_at(&self, pos: usize) -> String {
        let line = self.buffer.char_to_line(pos);
        let start = self.buffer.line_start(line);
        self.buffer
            .line(line)
            .chars()
            .take(pos - start)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    /// Indentation of a new line started at `pos`: the indentation of the
    /// line, one level more after a block opener (`:` in Python) or an
    /// unclosed bracket, one level less after a block-ending word.
    fn new_line_indent(&self, pos: usize) -> String {
        let mut indent = self.indent_at(pos);
        let line = self.buffer.char_to_line(pos);
        let start = self.buffer.line_start(line);
        let before: String = self.buffer.slice(start..pos);
        let trimmed = before.trim();
        let last = trimmed.chars().last();
        let opens_block = last.is_some_and(|c| {
            self.config.indent_after.contains(&c) || self.config.brackets.iter().any(|b| b.0 == c)
        });
        let first_word: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if opens_block {
            indent.push_str(&" ".repeat(self.config.indent));
        } else if !first_word.is_empty() && self.config.dedent_after.contains(&first_word) {
            let width = indent.chars().count();
            let keep = width.saturating_sub(self.config.indent);
            indent = indent.chars().take(keep).collect();
        }
        indent
    }

    fn newline_text(&self, pos: usize) -> String {
        format!("\n{}", self.new_line_indent(pos))
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Enter, cx) {
            return;
        }
        // Enter between an empty bracket pair puts the closing bracket on a
        // line of its own below the caret line.
        if self.selections.len() == 1 && self.selections.primary().is_empty() {
            let pos = self.selections.primary().head;
            let prev = pos.checked_sub(1).and_then(|p| self.buffer.char_at(p));
            let next = self.buffer.char_at(pos);
            if let (Some(p), Some(n)) = (prev, next)
                && self.config.brackets.contains(&(p, n))
            {
                let inner = self.newline_text(pos);
                let outer = format!("\n{}", self.indent_at(pos));
                let caret = pos + inner.chars().count();
                self.edit(
                    vec![Edit::insert(pos, format!("{inner}{outer}"))],
                    EditKind::Other,
                    CaretPlacement::Explicit(SelectionSet::caret(caret)),
                    cx,
                );
                return;
            }
        }
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), self.newline_text(s.start())))
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn start_new_line(&mut self, _: &StartNewLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Enter, cx) {
            return;
        }
        let edits = self
            .selections
            .iter()
            .map(|s| {
                let end = self.buffer.line_end(self.buffer.char_to_line(s.head));
                Edit::insert(end, self.newline_text(end))
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Lines touched by a selection (a selection ending at column 0 does not
    /// touch that line).
    fn selected_lines(&self, s: &Selection) -> Range<usize> {
        let first = self.buffer.char_to_line(s.start());
        let mut last = self.buffer.char_to_line(s.end());
        if last > first && self.buffer.offset_to_point(s.end()).column == 0 {
            last -= 1;
        }
        first..last + 1
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Tab, cx) {
            return;
        }
        let multiline = self
            .selections
            .iter()
            .any(|s| self.buffer.char_to_line(s.start()) != self.buffer.char_to_line(s.end()));
        if multiline {
            self.indent_lines(true, cx);
            return;
        }
        let tab = self.config.tab_size;
        let indent = self.config.indent;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                let p = self.buffer.offset_to_point(s.start());
                let col = movement::display_column(&self.buffer, p.line, p.column, tab);
                Edit::new(s.range(), " ".repeat(indent - col % indent))
            })
            .collect();
        self.edit(edits, EditKind::Typing, CaretPlacement::AfterEachEdit, cx);
    }

    fn unindent(&mut self, _: &Unindent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent_lines(false, cx);
    }

    /// Adds or removes one indentation level on every selected line; the
    /// selections grow and shrink with their lines.
    fn indent_lines(&mut self, add: bool, cx: &mut Context<Self>) {
        let indent = self.config.indent;
        let mut lines: Vec<usize> = self
            .selections
            .iter()
            .flat_map(|s| self.selected_lines(s))
            .collect();
        lines.dedup();
        let mut edits = Vec::new();
        for line in lines {
            let start = self.buffer.line_start(line);
            if add {
                if self.buffer.line_len(line) > 0 {
                    edits.push(Edit::insert(start, " ".repeat(indent)));
                }
            } else {
                let text = self.buffer.line(line);
                let n = if text.chars().next() == Some('\t') {
                    1
                } else {
                    text.chars().take(indent).take_while(|c| *c == ' ').count()
                };
                if n > 0 {
                    edits.push(Edit::delete(start..start + n));
                }
            }
        }
        // Selections follow the text: insertions at a line start do not move
        // a position at that start, deletions pull positions inside them to
        // their start.
        let map = |pos: usize| -> usize {
            let mut delta: isize = 0;
            for e in &edits {
                let (s, end) = (e.range.start, e.range.end);
                if e.text.is_empty() {
                    delta -= (pos.min(end).saturating_sub(s)) as isize;
                } else if s < pos {
                    delta += e.text.chars().count() as isize;
                }
            }
            (pos as isize + delta).max(0) as usize
        };
        let after = SelectionSet::from_vec(
            self.selections
                .iter()
                .map(|s| Selection::new(map(s.anchor), map(s.head)))
                .collect(),
        );
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Backspace, cx) {
            return;
        }
        let indent = self.config.indent;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                if self.is_empty_pair_at(s.head) {
                    return Edit::delete(s.head - 1..s.head + 1);
                }
                let p = self.buffer.offset_to_point(s.head);
                let line_start = s.head - p.column;
                let before = self.buffer.slice(line_start..s.head);
                if p.column > 0 && before.chars().all(|c| c == ' ') {
                    // In leading spaces: back to the previous indentation stop.
                    let target = (p.column - 1) / indent * indent;
                    return Edit::delete(line_start + target..s.head);
                }
                Edit::delete(movement::prev_grapheme(&self.buffer, s.head)..s.head)
            })
            .collect();
        self.edit(
            edits,
            EditKind::DeleteBackward,
            CaretPlacement::AfterEachEdit,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.forwarded(|| ForwardedInput::Delete, cx) {
            return;
        }
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(s.head..movement::next_grapheme(&self.buffer, s.head))
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(
            edits,
            EditKind::DeleteForward,
            CaretPlacement::AfterEachEdit,
            cx,
        );
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(movement::word_left(&self.buffer, s.head)..s.head)
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(s.head..movement::word_right(&self.buffer, s.head, stop))
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Char range of whole lines, including the line break after them (or
    /// before them for the last line).
    fn lines_range(&self, lines: Range<usize>) -> Range<usize> {
        let start = self.buffer.line_start(lines.start);
        if lines.end < self.buffer.len_lines() {
            start..self.buffer.line_start(lines.end)
        } else if start > 0 {
            start - 1..self.buffer.len_chars()
        } else {
            0..self.buffer.len_chars()
        }
    }

    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.caret_point();
        let primary = self.selected_lines(self.selections.primary());
        let mut ranges: Vec<Range<usize>> = self
            .selections
            .iter()
            .map(|s| self.lines_range(self.selected_lines(s)))
            .collect();
        ranges.sort_by_key(|r| r.start);
        ranges.dedup();
        // The caret keeps its column on the line that takes the place of the
        // deleted block (the line before it when the block was the last).
        let len_lines = self.buffer.len_lines();
        let target = if primary.end < len_lines {
            Some(primary.end)
        } else {
            primary.start.checked_sub(1)
        };
        let caret = target.map_or(0, |line| {
            let old = self.buffer.line_start(line);
            let removed: usize = ranges
                .iter()
                .map(|r| old.min(r.end).saturating_sub(r.start))
                .sum();
            old - removed + p.column.min(self.buffer.line_len(line))
        });
        let edits = ranges.into_iter().map(Edit::delete).collect();
        self.edit(
            edits,
            EditKind::Other,
            CaretPlacement::Explicit(SelectionSet::caret(caret)),
            cx,
        );
    }

    fn duplicate(&mut self, _: &DuplicateLine, _: &mut Window, cx: &mut Context<Self>) {
        let s = *self.selections.primary();
        if s.is_empty() {
            let line = self.buffer.char_to_line(s.head);
            let text = self.buffer.line_text(line).into_owned();
            let end = self.buffer.line_end(line);
            let caret = s.head + text.chars().count() + 1;
            self.edit(
                vec![Edit::insert(end, format!("\n{text}"))],
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::caret(caret)),
                cx,
            );
        } else {
            let text = self.buffer.slice(s.range());
            let len = s.range().len();
            let after = Selection::new(s.anchor + len, s.head + len);
            self.edit(
                vec![Edit::insert(s.end(), text)],
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::single(after)),
                cx,
            );
        }
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                let start = self.buffer.line_start(self.buffer.char_to_line(s.head));
                // At the line start: join with the previous line.
                Edit::delete(if start == s.head {
                    s.head.saturating_sub(1)..s.head
                } else {
                    start..s.head
                })
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                let end = self.buffer.line_end(self.buffer.char_to_line(s.head));
                // At the line end: join with the next line.
                Edit::delete(if end == s.head {
                    s.head..(s.head + 1).min(self.buffer.len_chars())
                } else {
                    s.head..end
                })
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Lines covered by all selections.
    fn all_selected_lines(&self) -> Range<usize> {
        let mut lines = self.selected_lines(self.selections.primary());
        for s in self.selections.iter() {
            let l = self.selected_lines(s);
            lines.start = lines.start.min(l.start);
            lines.end = lines.end.max(l.end);
        }
        lines
    }

    fn line_texts(&self, lines: Range<usize>) -> Vec<String> {
        lines
            .map(|l| self.buffer.line_text(l).into_owned())
            .collect()
    }

    /// Replaces whole lines `region` with `lines` and moves every
    /// selection by `shift` chars.
    fn replace_lines(
        &mut self,
        region: Range<usize>,
        lines: Vec<String>,
        shift: isize,
        cx: &mut Context<Self>,
    ) {
        let start = self.buffer.line_start(region.start);
        let end = self.buffer.line_end(region.end - 1);
        let moved = |p: usize| (p as isize + shift).max(0) as usize;
        let after = SelectionSet::from_vec(
            self.selections
                .iter()
                .map(|s| Selection::new(moved(s.anchor), moved(s.head)))
                .collect(),
        );
        self.edit(
            vec![Edit::new(start..end, lines.join("\n"))],
            EditKind::Other,
            CaretPlacement::Explicit(after),
            cx,
        );
    }

    /// Swaps the lines `block` with the line above or below.
    fn move_lines(&mut self, block: Range<usize>, up: bool, cx: &mut Context<Self>) {
        let n = self.buffer.len_lines();
        if (up && block.start == 0) || (!up && block.end >= n) {
            return;
        }
        let mut lines = self.line_texts(block.clone());
        if up {
            let above = self.buffer.line_text(block.start - 1).into_owned();
            let shift = -(above.chars().count() as isize + 1);
            lines.push(above);
            self.replace_lines(block.start - 1..block.end, lines, shift, cx);
        } else {
            let below = self.buffer.line_text(block.end).into_owned();
            let shift = below.chars().count() as isize + 1;
            lines.insert(0, below);
            self.replace_lines(block.start..block.end + 1, lines, shift, cx);
        }
    }

    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        let block = self.all_selected_lines();
        self.move_lines(block, true, cx);
    }

    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        let block = self.all_selected_lines();
        self.move_lines(block, false, cx);
    }

    /// Indentation width of a line, `None` for a blank line.
    fn line_indent(&self, line: usize) -> Option<usize> {
        let text = self.buffer.line_text(line);
        let tab = self.config.tab_size;
        if text.trim().is_empty() {
            return None;
        }
        let mut width = 0;
        for c in text.chars() {
            match c {
                ' ' => width += 1,
                '\t' => width += tab - width % tab,
                _ => break,
            }
        }
        Some(width)
    }

    fn starts_with_closer(&self, line: usize) -> bool {
        let text = self.buffer.line_text(line);
        let first = text.trim_start().chars().next();
        first.is_some_and(|c| self.config.brackets.iter().any(|b| b.1 == c))
    }

    /// End (exclusive) of the statement starting at `line`: the line and
    /// the following lines indented deeper (or closing a bracket at the same
    /// indentation), without trailing blank lines.
    fn statement_end(&self, line: usize) -> usize {
        let Some(indent) = self.line_indent(line) else {
            return line + 1;
        };
        let mut end = line + 1;
        for l in line + 1..self.buffer.len_lines() {
            match self.line_indent(l) {
                None => continue,
                Some(i) if i > indent || (i == indent && self.starts_with_closer(l)) => end = l + 1,
                Some(_) => break,
            }
        }
        end
    }

    /// Start of the statement before `line` at the same indentation.
    fn previous_statement(&self, line: usize, indent: usize) -> Option<usize> {
        for l in (0..line).rev() {
            match self.line_indent(l) {
                None => continue,
                Some(i) if i > indent || (i == indent && self.starts_with_closer(l)) => continue,
                Some(i) if i == indent => return Some(l),
                Some(_) => return None,
            }
        }
        None
    }

    /// Start of the statement after the one ending at `end`.
    fn next_statement(&self, end: usize, indent: usize) -> Option<usize> {
        (end..self.buffer.len_lines())
            .find_map(|l| self.line_indent(l).map(|i| (l, i)))
            .and_then(|(l, i)| (i == indent).then_some(l))
    }

    /// Swaps the statement at the caret (with its block) with the previous
    /// or next statement of the same indentation; blank lines between them
    /// stay where they are. On a blank line it moves the line.
    fn move_statement(&mut self, up: bool, cx: &mut Context<Self>) {
        let lines = self.all_selected_lines();
        let Some(indent) = self.line_indent(lines.start) else {
            self.move_lines(lines, up, cx);
            return;
        };
        let end = self.statement_end(lines.start).max(lines.end);
        let block = self.line_texts(lines.start..end);
        if up {
            let Some(prev) = self.previous_statement(lines.start, indent) else {
                return;
            };
            let prev_end = self.statement_end(prev);
            let other = self.line_texts(prev..prev_end);
            let gap = self.line_texts(prev_end..lines.start);
            let shift = -(self.buffer.line_start(lines.start) as isize
                - self.buffer.line_start(prev) as isize);
            let mut out = block;
            out.extend(gap);
            out.extend(other);
            self.replace_lines(prev..end, out, shift, cx);
        } else {
            let Some(next) = self.next_statement(end, indent) else {
                return;
            };
            let next_end = self.statement_end(next);
            let other = self.line_texts(next..next_end);
            let gap = self.line_texts(end..next);
            let moved: usize = other
                .iter()
                .chain(&gap)
                .map(|l| l.chars().count() + 1)
                .sum();
            let mut out = other;
            out.extend(gap);
            out.extend(block);
            self.replace_lines(lines.start..next_end, out, moved as isize, cx);
        }
    }

    fn move_statement_up(&mut self, _: &MoveStatementUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_statement(true, cx);
    }

    fn move_statement_down(
        &mut self,
        _: &MoveStatementDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_statement(false, cx);
    }

    /// Joins the next line to the caret line, or all selected lines: the
    /// line break and the indentation after it become one space (none
    /// next to brackets or blank lines).
    fn join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        let lines = self.selected_lines(self.selections.primary());
        let last = if lines.len() > 1 {
            lines.end - 1
        } else {
            lines.start + 1
        };
        if last >= self.buffer.len_lines() {
            return;
        }
        let mut edits = Vec::new();
        for l in lines.start..last {
            let text = self.buffer.line_text(l);
            let next = self.buffer.line_text(l + 1);
            let trimmed = text.trim_end();
            let cut_start = self.buffer.line_start(l) + trimmed.chars().count();
            let next_start = self.buffer.line_start(l + 1);
            let cut_end = next_start + leading_ws(&next);
            let opens = trimmed
                .chars()
                .last()
                .is_some_and(|c| self.closing_bracket(c).is_some());
            let closes = next
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| self.config.brackets.iter().any(|b| b.1 == c));
            let sep =
                if trimmed.trim_start().is_empty() || next.trim().is_empty() || opens || closes {
                    ""
                } else {
                    " "
                };
            edits.push(Edit::new(cut_start..cut_end, sep));
        }
        let first = &edits[0];
        let caret = first.range.start;
        let caret = SelectionSet::caret(caret);
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(caret), cx);
    }

    /// Comments the selected lines (or the caret line) with the file type's
    /// line comment at their smallest indentation, or uncomments them when
    /// every non-blank line is commented. Without a selection the caret
    /// moves to the next line.
    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prefix) = self.config.line_comment.clone() else {
            return;
        };
        let marker = prefix.trim_end().to_string();
        let lines = self.all_selected_lines();
        let texts: Vec<(usize, String)> = lines
            .clone()
            .map(|l| (l, self.buffer.line_text(l).into_owned()))
            .collect();
        let code: Vec<&(usize, String)> =
            texts.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
        let commented = !code.is_empty()
            && code
                .iter()
                .all(|(_, t)| t.trim_start().starts_with(&marker));
        let mut edits = Vec::new();
        if commented {
            for (l, t) in &code {
                let at = self.buffer.line_start(*l) + leading_ws(t);
                let rest = &t.trim_start()[marker.len()..];
                let space = usize::from(rest.starts_with(' ') && prefix.ends_with(' '));
                edits.push(Edit::delete(at..at + marker.chars().count() + space));
            }
        } else {
            let targets: Vec<&(usize, String)> = if code.is_empty() {
                texts.iter().collect()
            } else {
                code
            };
            let indent = targets
                .iter()
                .map(|(_, t)| leading_ws(t))
                .min()
                .unwrap_or(0);
            for (l, _) in targets {
                edits.push(Edit::insert(self.buffer.line_start(*l) + indent, &prefix));
            }
        }
        let single_caret = self.selections.len() == 1 && self.selections.primary().is_empty();
        let after = if single_caret && lines.end < self.buffer.len_lines() {
            // The caret goes down one line, keeping its column.
            let column = self.caret_point().column;
            let next_start = map_through(&edits, self.buffer.line_start(lines.end));
            let next_len = self.buffer.line_len(lines.end);
            SelectionSet::caret(next_start + column.min(next_len))
        } else {
            SelectionSet::from_vec(
                self.selections
                    .iter()
                    .map(|s| {
                        Selection::new(map_through(&edits, s.anchor), map_through(&edits, s.head))
                    })
                    .collect(),
            )
        };
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_or_ask(false, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_or_ask(true, cx);
    }

    // ---- clipboard ------------------------------------------------------------

    /// Text to copy and whether it is whole lines (nothing selected).
    fn copy_text(&self) -> (String, bool) {
        if self.selections.iter().all(|s| s.is_empty()) {
            let mut text = String::new();
            let mut last_line = None;
            for s in self.selections.iter() {
                let line = self.buffer.char_to_line(s.head);
                if last_line == Some(line) {
                    continue;
                }
                last_line = Some(line);
                text.push_str(&self.buffer.line_text(line));
                text.push('\n');
            }
            (text, true)
        } else {
            let parts: Vec<String> = self
                .selections
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| self.buffer.slice(s.range()))
                .collect();
            (parts.join("\n"), false)
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let (text, whole_lines) = self.copy_text();
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            LineCopy { whole_lines },
        ));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let (text, whole_lines) = self.copy_text();
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            LineCopy { whole_lines },
        ));
        if whole_lines {
            self.delete_line(&DeleteLine, window, cx);
        } else {
            let edits = self
                .selections
                .iter()
                .map(|s| Edit::delete(s.range()))
                .collect();
            self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else {
            return;
        };
        if self.forwarded(
            || ForwardedInput::Text(text::normalize_line_endings(&text).into()),
            cx,
        ) {
            return;
        }
        let whole_lines = item.entries().iter().any(|e| {
            matches!(e, ClipboardEntry::String(s)
                if s.metadata_json::<LineCopy>().is_some_and(|m| m.whole_lines))
        });
        let text = text::normalize_line_endings(&text).into_owned();
        if whole_lines && self.selections.iter().all(|s| s.is_empty()) {
            // Whole lines go above the caret line; the caret stays on its text.
            let len = text.chars().count();
            let edits: Vec<Edit> = self
                .selections
                .iter()
                .map(|s| {
                    Edit::insert(
                        self.buffer.line_start(self.buffer.char_to_line(s.head)),
                        text.clone(),
                    )
                })
                .collect();
            let after = SelectionSet::from_vec(
                self.selections
                    .iter()
                    .enumerate()
                    .map(|(i, s)| Selection::caret(s.head + len * (i + 1)))
                    .collect(),
            );
            self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
            return;
        }
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), text.clone()))
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    // ---- mouse ----------------------------------------------------------------

    /// Mouse press at char offset `pos` with `clicks` (1, 2, 3); `lean`:
    /// the press was at the end of a row before a soft wrap at `pos`.
    pub(crate) fn mouse_down(
        &mut self,
        pos: usize,
        lean: bool,
        clicks: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let s = *self.selections.primary();
        match clicks {
            2 => {
                let word = movement::word_range(&self.buffer, pos);
                self.drag = Some(DragUnit::Word(word.clone()));
                self.set_selections(
                    SelectionSet::single(Selection::new(word.start, word.end)),
                    cx,
                );
            }
            n if n >= 3 => {
                let line = self.buffer.char_to_line(pos);
                let range = self.lines_range_with_break(line);
                self.drag = Some(DragUnit::Line(range.clone()));
                self.set_selections(
                    SelectionSet::single(Selection::new(range.start, range.end)),
                    cx,
                );
            }
            _ => {
                self.drag = Some(DragUnit::Char);
                let sel = if shift {
                    Selection::new(s.anchor, pos)
                } else {
                    Selection::caret(pos)
                };
                self.set_selections(SelectionSet::single(sel), cx);
                if lean {
                    self.lean_back.insert(pos);
                }
                // A click elsewhere without a selection is a navigation.
                if !shift && s.is_empty() && s.head != pos {
                    self.jumped_from(s.head, cx);
                }
            }
        }
    }

    fn lines_range_with_break(&self, line: usize) -> Range<usize> {
        let start = self.buffer.line_start(line);
        let end = if line + 1 < self.buffer.len_lines() {
            self.buffer.line_start(line + 1)
        } else {
            self.buffer.len_chars()
        };
        start..end
    }

    /// Mouse drag to char offset `pos` (`lean`: to the end of a row before
    /// a soft wrap there).
    pub(crate) fn mouse_drag(&mut self, pos: usize, lean: bool, cx: &mut Context<Self>) {
        let s = *self.selections.primary();
        let sel = match self.drag.clone() {
            Some(DragUnit::Char) => Selection::new(s.anchor, pos),
            Some(DragUnit::Word(origin)) => {
                let word = movement::word_range(&self.buffer, pos);
                if pos < origin.start {
                    Selection::new(origin.end, word.start.min(pos))
                } else {
                    Selection::new(origin.start, word.end.max(pos))
                }
            }
            Some(DragUnit::Line(origin)) => {
                let line = self.lines_range_with_break(self.buffer.char_to_line(pos));
                if line.start < origin.start {
                    Selection::new(origin.end, line.start)
                } else {
                    Selection::new(origin.start, line.end)
                }
            }
            _ => return,
        };
        let lean = lean && sel.head == pos && matches!(self.drag, Some(DragUnit::Char));
        if sel != s || lean != self.leans_back(pos) {
            self.set_selections(SelectionSet::single(sel), cx);
            if lean {
                self.lean_back.insert(pos);
            }
        }
    }

    pub(crate) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// Scrolls by a wheel or trackpad delta (positive y scrolls down).
    /// Soft-wrapped text does not scroll sideways.
    pub(crate) fn scroll_by(&mut self, delta: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        self.hide_read_only_hint(cx);
        let max_y = self.max_scroll_y();
        self.scroll.y = (self.scroll.y + delta.y).clamp(px(0.0), max_y);
        if self.soft_wrap.is_none() {
            self.scroll.x = (self.scroll.x + delta.x).max(px(0.0));
        }
        cx.notify();
    }

    pub(crate) fn max_scroll_y(&self) -> Pixels {
        let height = self
            .layout
            .as_ref()
            .map_or(px(0.0), |l| l.bounds.size.height);
        self.max_scroll_y_for(height)
    }

    /// How far the text scrolls in a view of `view_height`: until the last
    /// row is at the top, or for a console until it is at the bottom.
    pub(crate) fn max_scroll_y_for(&self, view_height: Pixels) -> Pixels {
        let line_height = self.row_height();
        let rows = self.display_rows();
        if self.console {
            (line_height * rows as f32 - view_height).max(px(0.0))
        } else {
            line_height * rows.saturating_sub(1) as f32
        }
    }

    pub(crate) fn set_scroll_y(&mut self, y: Pixels, cx: &mut Context<Self>) {
        self.scroll.y = y.clamp(px(0.0), self.max_scroll_y());
        cx.notify();
    }
}

// ---- navigation history -------------------------------------------------

impl Editor {
    /// How far line `line` (its first row) is from the top of the view, as
    /// of the last frame (`None` before the first).
    pub fn line_view_y(&self, line: usize) -> Option<Pixels> {
        let line_height = self.layout.as_ref()?.line_height;
        Some(line_height * self.line_row(line) as f32 - self.scroll.y)
    }

    /// How far the primary caret's row is from the top of the view (the
    /// reference's relative caret position).
    pub fn caret_view_y(&self) -> Option<Pixels> {
        let line_height = self.layout.as_ref()?.line_height;
        Some(line_height * self.primary_row() as f32 - self.scroll.y)
    }

    /// Reports that the caret jumped from char offset `from` as a
    /// navigation ([`EditorEvent::Jumped`]): a component that moves the
    /// caret as a navigation calls it after the move.
    pub fn jumped_from(&mut self, from: usize, cx: &mut Context<Self>) {
        let from = self
            .buffer
            .offset_to_point(from.min(self.buffer.len_chars()));
        let view_y = self.line_view_y(from.line);
        cx.emit(EditorEvent::Jumped { from, view_y });
    }

    /// Puts the caret at a zero-based point (clamped) with its line
    /// `view_y` from the top of the view, or in the middle of the view
    /// without it; the caret stays visible (going back to a place, as the
    /// reference restores a text editor's state).
    pub fn go_to_place(&mut self, point: Point, view_y: Option<Pixels>, cx: &mut Context<Self>) {
        self.go_to(point.line + 1, Some(point.column + 1), cx);
        if view_y.is_some() {
            self.center_caret = false;
            self.caret_view_y = view_y;
        }
    }

    /// Scrolls the caret line to the place's position in the view (before
    /// the caret is kept visible; `line_height` of this frame).
    pub(crate) fn apply_caret_view_y(&mut self, line_height: Pixels) {
        if let Some(y) = self.caret_view_y.take() {
            self.scroll.y = line_height * self.primary_row() as f32 - y;
        }
    }
}

macro_rules! editor_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl Editor {
            /// The editor's own action handlers.
            fn bind_actions(el: Div, cx: &mut Context<Self>) -> Div {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Adds handlers to `el` that forward every editor command to
        /// `editor`; the workbench uses this so commands reach the active
        /// editor when another component has the focus.
        pub fn forward_actions<E: InteractiveElement>(el: E, editor: Entity<Editor>) -> E {
            el $(.on_action({
                let editor = editor.clone();
                move |a: &$action, window, cx| editor.update(cx, |e, cx| e.$method(a, window, cx))
            }))*
        }
    };
}

editor_actions! {
    MoveLeft => move_left,
    MoveRight => move_right,
    MoveUp => move_up,
    MoveDown => move_down,
    MoveWordLeft => move_word_left,
    MoveWordRight => move_word_right,
    MoveLineStart => move_line_start,
    MoveLineEnd => move_line_end,
    MoveDocumentStart => move_document_start,
    MoveDocumentEnd => move_document_end,
    PageUp => page_up,
    PageDown => page_down,
    SelectLeft => select_left,
    SelectRight => select_right,
    SelectUp => select_up,
    SelectDown => select_down,
    SelectWordLeft => select_word_left,
    SelectWordRight => select_word_right,
    SelectLineStart => select_line_start,
    SelectLineEnd => select_line_end,
    SelectDocumentStart => select_document_start,
    SelectDocumentEnd => select_document_end,
    SelectPageUp => select_page_up,
    SelectPageDown => select_page_down,
    SelectAll => select_all,
    SelectWord => select_word,
    SetSelection => set_selection,
    Type => type_text,
    Newline => newline,
    StartNewLine => start_new_line,
    Tab => tab,
    Unindent => unindent,
    Backspace => backspace,
    Delete => delete,
    DeleteWordLeft => delete_word_left,
    DeleteWordRight => delete_word_right,
    DeleteLine => delete_line,
    DuplicateLine => duplicate,
    Undo => undo,
    Redo => redo,
    Copy => copy,
    Cut => cut,
    Paste => paste,
    ShrinkSelection => shrink_selection,
    Escape => escape,
    DeleteToLineStart => delete_to_line_start,
    DeleteToLineEnd => delete_to_line_end,
    MoveLineUp => move_line_up,
    MoveLineDown => move_line_down,
    MoveStatementUp => move_statement_up,
    MoveStatementDown => move_statement_down,
    JoinLines => join_lines,
    ToggleComment => toggle_comment,
    GoToLine => go_to_line,
    ScrollToLine => scroll_to_line,
    NextProblem => next_problem,
    PreviousProblem => previous_problem,
    ToggleSoftWraps => toggle_soft_wraps,
    SampleDecorations => sample_decorations,
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let widget = crate::inspection::widget(self, cx);
        let tooltip = crate::inspection::stripe_tooltip(self, cx);
        let el = div()
            .key_context(if self.single_line {
                "Editor single_line"
            } else {
                "Editor"
            })
            .track_focus(&self.focus)
            // On the focused element itself: GPUI sends modifier changes
            // along the focus path only.
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, _, cx| {
                this.modifiers_changed(e.modifiers, cx)
            }))
            .size_full()
            .relative()
            .child(EditorElement::new(cx.entity()))
            .children(widget)
            .children(tooltip)
            .children(crate::viewer::render_read_only_hint(self, cx));
        Self::bind_actions(el, cx)
    }
}

/// Opens an editor for `text` (convenience for tests and tools).
pub fn new_editor(text: &str, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    cx.new(|cx| Editor::new(Buffer::new(text), window, cx))
}
