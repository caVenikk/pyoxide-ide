//! Recent Files (Cmd+E / Ctrl+E), as the reference IDE's: the tool windows
//! on the left, the recently opened files on the right (the current one
//! first, the one before it selected), "Show edited only" for the files
//! changed in this session. Typing filters both lists (speed search);
//! Enter opens, Delete or Backspace takes a file off the list and closes
//! its tab, Left and Right go between the lists; pressing the shortcut
//! again switches "Show edited only". With a filter that matches nothing
//! Enter opens Go to File with the text typed.

use std::path::{Path, PathBuf};

use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Render, SharedString, StyledText, Subscription, UniformListScrollHandle, Window,
    div, prelude::*, px, uniform_list,
};
use name_match::{Matcher, MatchingMode};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::components::checkbox;
use ui::icons::Icon;
use ui::theme::ThemeExt;

use crate::file_types::FileTypes;
use crate::popup::{PopupFrame, PopupPlace};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Recent Files.
    Open => "recent-files.open", "Recent Files",
        "Shows the recently opened files and the tool windows; again while it is open, switches \"Show edited only\".";
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "RecentFiles";
    /// Close.
    Close => "recent-files.close", "Close", "Clears the speed search, else closes Recent Files.";
    /// The next row.
    Down => "recent-files.down", "Next", "Selects the next row of the list.";
    /// The previous row.
    Up => "recent-files.up", "Previous", "Selects the previous row of the list.";
    /// The tool windows.
    Left => "recent-files.left", "Tool Windows", "Moves to the list of tool windows.";
    /// The files.
    Right => "recent-files.right", "Files", "Moves to the list of files.";
    /// Open the selected row.
    Select => "recent-files.select", "Open", "Opens the selected file or tool window.";
    /// Delete.
    Delete => "recent-files.delete", "Delete from Recent Files",
        "Deletes the last char of the speed search, else takes the selected file off the list and closes its tab.";
}

/// Smallest size (the reference's medium dialog).
const MIN_WIDTH: f32 = 500.0;
const MIN_HEIGHT: f32 = 350.0;
const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;
const HEADER_HEIGHT: f32 = 36.0;
const FOOTER_HEIGHT: f32 = 26.0;
const TOOL_WINDOWS_WIDTH: f32 = 170.0;

/// A tool window the popup lists.
#[derive(Debug, Clone)]
pub struct ToolWindowItem {
    pub id: &'static str,
    pub title: &'static str,
    pub shortcut: Option<SharedString>,
}

/// What the popup opens with.
#[derive(Debug, Clone, Default)]
pub struct OpenContext {
    pub root: Option<PathBuf>,
    /// Recently opened files, newest first (the current one first).
    pub viewed: Vec<PathBuf>,
    /// Files changed in this session, newest first.
    pub edited: Vec<PathBuf>,
    pub current: Option<PathBuf>,
    pub tool_windows: Vec<ToolWindowItem>,
    /// The shortcut of Recent Files (shown next to "Show edited only").
    pub shortcut: Option<SharedString>,
}

#[derive(Debug, Clone)]
pub enum RecentFilesEvent {
    OpenFile(PathBuf),
    /// Show a tool window.
    ShowToolWindow(&'static str),
    /// Take a file off the recent files (and close its tab).
    Remove { path: PathBuf, edited: bool },
    /// Go to File with a text (nothing matched it here).
    GoToFile(String),
    /// Closed with Escape: the focus goes back.
    Dismissed,
}

impl EventEmitter<RecentFilesEvent> for RecentFiles {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    ToolWindows,
    Files,
}

pub struct RecentFiles {
    focus: FocusHandle,
    file_types: std::rc::Rc<FileTypes>,
    open: bool,
    context: OpenContext,
    edited_only: bool,
    filter: String,
    list: List,
    selected: usize,
    files_scroll: UniformListScrollHandle,
    frame: PopupFrame,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for RecentFiles {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RecentFiles {
    pub fn new(file_types: std::rc::Rc<FileTypes>, place: Option<PopupPlace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![cx.on_focus_out(&focus, window, |this, _, window, cx| {
            if this.open && window.is_window_active() {
                this.close(false, cx);
            }
        })];
        let mut frame = PopupFrame::new(
            gpui::size(px(WIDTH), px(HEIGHT)),
            gpui::size(px(MIN_WIDTH), px(MIN_HEIGHT)),
            px(0.0),
        );
        if let Some(place) = place {
            frame.restore(place);
        }
        Self {
            focus,
            file_types,
            open: false,
            context: OpenContext::default(),
            edited_only: false,
            filter: String::new(),
            list: List::Files,
            selected: 0,
            files_scroll: UniformListScrollHandle::new(),
            frame,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Where the popup was left and how large it was.
    pub fn place(&self) -> Option<PopupPlace> {
        self.frame.place()
    }

    /// Opens the popup, or switches "Show edited only" when it is open.
    pub fn show(&mut self, context: OpenContext, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.edited_only = !self.edited_only;
            self.select_first(cx);
            return;
        }
        self.context = context;
        self.open = true;
        self.edited_only = false;
        self.filter.clear();
        self.select_first(cx);
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn close(&mut self, restore_focus: bool, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.frame.cancel_drag();
        let place = self.frame.place();
        if place.is_some() {
            crate::search_everywhere::update_settings(cx, |s| s.recent_files = place);
        }
        if restore_focus {
            cx.emit(RecentFilesEvent::Dismissed);
        }
        cx.notify();
    }

    /// The files of the list (all, or the edited ones), filtered.
    fn files(&self) -> Vec<PathBuf> {
        let source = if self.edited_only {
            &self.context.edited
        } else {
            &self.context.viewed
        };
        let mut seen = std::collections::HashSet::new();
        source
            .iter()
            .filter(|p| seen.insert((*p).clone()))
            .filter(|p| self.filter.is_empty() || self.matches(&self.row_text(p)).is_some())
            .cloned()
            .collect()
    }

    fn tool_windows(&self) -> Vec<ToolWindowItem> {
        self.context
            .tool_windows
            .iter()
            .filter(|t| self.filter.is_empty() || self.matches(t.title).is_some())
            .cloned()
            .collect()
    }

    /// The text speed search matches: the name, and the folder when shown.
    fn row_text(&self, path: &Path) -> String {
        let name = file_name(path);
        match self.location(path) {
            Some(location) => format!("{name} {location}"),
            None => name,
        }
    }

    /// A file's folder (relative to the project), shown when another
    /// listed file has the same name.
    fn location(&self, path: &Path) -> Option<String> {
        let name = file_name(path);
        let twins = self
            .context
            .viewed
            .iter()
            .chain(&self.context.edited)
            .filter(|p| *p != path && file_name(p) == name)
            .count();
        if twins == 0 {
            return None;
        }
        let parent = path.parent()?;
        let rel = self
            .context
            .root
            .as_deref()
            .and_then(|r| parent.strip_prefix(r).ok())
            .unwrap_or(parent);
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    /// The speed search's match in a text: its fragments and degree.
    fn matches(&self, text: &str) -> Option<name_match::Match> {
        let matcher = Matcher::new(&format!("*{}", self.filter), MatchingMode::IgnoreCase);
        let chars: Vec<char> = text.chars().collect();
        matcher.match_chars(&chars)
    }

    /// The first selection: the file before the current one (the current
    /// is first), or with a filter the best match.
    fn select_first(&mut self, cx: &mut Context<Self>) {
        let files = self.files();
        self.list = List::Files;
        self.selected = if !self.filter.is_empty() {
            self.best_match(&files)
        } else if files.first() == self.context.current.as_ref() && files.len() > 1 {
            1
        } else {
            0
        };
        if files.is_empty() && !self.tool_windows().is_empty() && !self.filter.is_empty() {
            self.list = List::ToolWindows;
            self.selected = 0;
        }
        cx.notify();
    }

    fn best_match(&self, files: &[PathBuf]) -> usize {
        files
            .iter()
            .enumerate()
            .max_by_key(|(i, p)| {
                let degree = self.matches(&self.row_text(p)).map_or(i32::MIN, |m| m.degree);
                (degree, std::cmp::Reverse(*i))
            })
            .map_or(0, |(i, _)| i)
    }

    fn rows(&self) -> usize {
        match self.list {
            List::Files => self.files().len(),
            List::ToolWindows => self.tool_windows().len(),
        }
    }

    pub fn state_json(&self) -> serde_json::Value {
        let rel = |p: &Path| {
            self.context
                .root
                .as_deref()
                .and_then(|r| p.strip_prefix(r).ok())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        };
        json!({
            "open": self.open,
            "edited_only": self.edited_only,
            "filter": self.filter,
            "list": if self.list == List::Files { "files" } else { "tool_windows" },
            "selected": self.selected,
            "files": self.files().iter().map(|p| rel(p)).collect::<Vec<_>>(),
            "tool_windows": self.tool_windows().iter().map(|t| t.title).collect::<Vec<_>>(),
        })
    }

    // ---- actions ----------------------------------------------------------

    fn close_action(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.select_first(cx);
            return;
        }
        self.close(true, cx);
    }

    fn down_action(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows();
        if n > 0 {
            self.selected = (self.selected + 1) % n;
            self.reveal(cx);
        }
    }

    fn up_action(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows();
        if n > 0 {
            self.selected = (self.selected + n - 1) % n;
            self.reveal(cx);
        }
    }

    fn left_action(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_list(List::ToolWindows, cx);
    }

    fn right_action(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_list(List::Files, cx);
    }

    fn switch_list(&mut self, list: List, cx: &mut Context<Self>) {
        if self.list == list {
            return;
        }
        let n = match list {
            List::Files => self.files().len(),
            List::ToolWindows => self.tool_windows().len(),
        };
        if n == 0 {
            return;
        }
        self.list = list;
        self.selected = self.selected.min(n - 1);
        cx.notify();
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if self.list == List::Files {
            self.files_scroll
                .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn select_action(&mut self, _: &Select, _: &mut Window, cx: &mut Context<Self>) {
        self.activate(self.list, self.selected, cx);
    }

    fn activate(&mut self, list: List, index: usize, cx: &mut Context<Self>) {
        match list {
            List::Files => {
                let files = self.files();
                match files.get(index) {
                    Some(path) => {
                        let path = path.clone();
                        self.close(false, cx);
                        cx.emit(RecentFilesEvent::OpenFile(path));
                    }
                    None if !self.filter.is_empty() && self.tool_windows().is_empty() => {
                        let text = self.filter.clone();
                        self.close(false, cx);
                        cx.emit(RecentFilesEvent::GoToFile(text));
                    }
                    None => {}
                }
            }
            List::ToolWindows => {
                if let Some(t) = self.tool_windows().get(index) {
                    let id = t.id;
                    self.close(false, cx);
                    cx.emit(RecentFilesEvent::ShowToolWindow(id));
                }
            }
        }
    }

    fn delete_action(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if !self.filter.is_empty() {
            self.filter.pop();
            self.select_first(cx);
            return;
        }
        if self.list != List::Files {
            return;
        }
        let files = self.files();
        let Some(path) = files.get(self.selected).cloned() else {
            return;
        };
        let edited = self.edited_only;
        self.context.viewed.retain(|p| *p != path);
        if edited {
            self.context.edited.retain(|p| *p != path);
        }
        let n = self.files().len();
        self.selected = self.selected.min(n.saturating_sub(1));
        cx.emit(RecentFilesEvent::Remove { path, edited });
        cx.notify();
    }

    /// A typed char goes to the speed search.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform || k.modifiers.alt || k.modifiers.function {
            return;
        }
        let Some(text) = k.key_char.as_ref().filter(|t| !t.is_empty() && !t.chars().any(char::is_control)) else {
            return;
        };
        self.filter.push_str(text);
        self.select_first(cx);
        cx.stop_propagation();
    }

    fn toggle_edited(&mut self, cx: &mut Context<Self>) {
        self.edited_only = !self.edited_only;
        self.select_first(cx);
    }

    // ---- drawing ----------------------------------------------------------

    fn render_file(&self, index: usize, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.list == List::Files && self.selected == index;
        let name = file_name(path);
        let location = self.location(path);
        let match_bg = theme.color(UiRole::SearchMatch);
        let ranges = if self.filter.is_empty() {
            Vec::new()
        } else {
            self.matches(&name).map(|m| m.fragments).unwrap_or_default()
        };
        div()
            .id(("recent-files-file", index))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(theme.metric(Metric::RowHeight))
            .px(px(8.0))
            .rounded(theme.metric(Metric::CornerRadius))
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| d.bg(theme.color(UiRole::Selection)))
            .child(Icon::new(self.file_types.icon(path)).size(px(16.0)))
            .child(StyledText::new(name.clone()).with_highlights(highlight(&name, &ranges, match_bg)))
            .children(location.map(|l| div().text_color(theme.color(UiRole::TextMuted)).child(l)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate(List::Files, index, cx)))
            .into_any_element()
    }
}

use gpui::AnyElement;

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Char ranges of `text` as byte ranges with a background.
fn highlight(
    text: &str,
    ranges: &[std::ops::Range<usize>],
    background: gpui::Hsla,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let style = HighlightStyle {
        background_color: Some(background),
        ..HighlightStyle::default()
    };
    ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.min(offsets.len() - 1);
            let b = r.end.min(offsets.len() - 1);
            (b > a).then(|| (offsets[a]..offsets[b], style))
        })
        .collect()
}

impl Render for RecentFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        // Centred in the window.
        let viewport = window.viewport_size();
        let height = self.frame.place().map_or(px(HEIGHT), |p| px(p.height));
        self.frame.set_top(((viewport.height - height) / 2.0).max(px(16.0)));
        let files = self.files();
        let tools = self.tool_windows();
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let selected_path = match self.list {
            List::Files => files.get(self.selected).and_then(|p| p.parent()).map(home_relative),
            List::ToolWindows => tools.get(self.selected).map(|t| format!("Activate tool window: {}", t.title)),
        };
        let header = div()
            .id("recent-files-header")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(HEADER_HEIGHT))
            .px(px(12.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.frame.start_move(e.position);
                    cx.notify();
                }),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Recent Files"))
            .child(div().flex_1())
            .child(
                div()
                    .id("recent-files-edited")
                    .debug_selector(|| "recent-files-edited".into())
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(checkbox("recent-files-edited-box", self.edited_only, cx))
                    .child("Show edited only")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_edited(cx))),
            )
            .children(self.context.shortcut.clone().map(|s| div().text_color(muted).child(s)));
        let tool_rows: Vec<AnyElement> = tools
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let selected = self.list == List::ToolWindows && self.selected == i;
                div()
                    .id(("recent-files-tool", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(theme.metric(Metric::RowHeight))
                    .px(px(8.0))
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected, |d| d.bg(theme.color(UiRole::Selection)))
                    .child(div().flex_1().child(t.title))
                    .children(t.shortcut.clone().map(|s| div().text_color(muted).child(s)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate(List::ToolWindows, i, cx)))
                    .into_any_element()
            })
            .collect();
        let file_list: AnyElement = if files.is_empty() {
            let empty = if self.filter.is_empty() {
                "No recent files"
            } else {
                "Press 'Enter' to search in Project"
            };
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(empty)
                .into_any_element()
        } else {
            uniform_list(
                "recent-files-files",
                files.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let files = this.files();
                    range
                        .filter_map(|i| files.get(i).map(|p| this.render_file(i, p, cx)))
                        .collect::<Vec<_>>()
                }),
            )
            .debug_selector(|| "recent-files-files".into())
            .track_scroll(self.files_scroll.clone())
            .flex_1()
            .into_any_element()
        };
        let theme = cx.theme();
        let content = div()
            .id("recent-files")
            .key_context("RecentFiles")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .on_key_down(cx.listener(Self::key_down))
            .child(header)
            .when(!self.filter.is_empty(), |d| {
                d.child(
                    div()
                        .id("recent-files-speed-search")
                        .debug_selector(|| "recent-files-speed-search".into())
                        .flex_none()
                        .mx(px(12.0))
                        .px(px(6.0))
                        .rounded(theme.metric(Metric::CornerRadius))
                        .bg(theme.color(UiRole::SearchField))
                        .child(format!("Search for: {}", self.filter)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(4.0))
                    .when(!tools.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_none()
                                .w(px(TOOL_WINDOWS_WIDTH))
                                .pr(px(4.0))
                                .border_r_1()
                                .border_color(theme.color(UiRole::Divider))
                                .children(tool_rows),
                        )
                    })
                    .child(div().flex().flex_col().flex_1().min_w(px(0.0)).pl(px(4.0)).child(file_list)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(FOOTER_HEIGHT))
                    .px(px(12.0))
                    .text_size(px(11.0))
                    .text_color(muted)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(selected_path.unwrap_or_default()),
            );
        let content = content
            .on_action(cx.listener(Self::close_action))
            .on_action(cx.listener(Self::down_action))
            .on_action(cx.listener(Self::up_action))
            .on_action(cx.listener(Self::left_action))
            .on_action(cx.listener(Self::right_action))
            .on_action(cx.listener(Self::select_action))
            .on_action(cx.listener(Self::delete_action));
        self.frame
            .render("recent-files", content, |this| &mut this.frame, window, cx)
            .into_any_element()
    }
}

/// A folder relative to the home folder (`~/...`).
fn home_relative(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
    }
    path.to_string_lossy().into_owned()
}

/// Registers the commands.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
}
