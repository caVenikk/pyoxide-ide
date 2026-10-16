//! Notification balloons, as the reference IDE shows them in the New UI
//! (IntelliJ Community, Apache-2.0: `NotificationsManagerImpl.createBalloon`,
//! `BalloonLayoutConfiguration`, `ActionCenterBalloonLayout`,
//! `NotificationBalloonActionProvider`, `BalloonImpl`'s smart fade-out).
//!
//! A balloon has a kind with its icon, a title (bold when there is
//! content), a content text, action links and a close button shown while
//! the mouse is over it. An ordinary balloon goes after 10 s, a sticky one
//! (the reference's `STICKY_BALLOON` display type, used for errors) after
//! 5 min; the time runs from the first input after it was shown, stops
//! while the mouse is over the balloon and while the window is inactive.
//! At most three are shown, the newest on top; a fourth collapses the
//! oldest into a "1 more notification" strip below the stack.
//!
//! Content that does not fit on a line of 320 px wraps at 240 px (the
//! `width:240px` style of `NotificationsUtil.buildHtml`); a balloon shows
//! four lines less one for a title and one for action links, and a
//! chevron at the right of the last line expands it to the whole text
//! (ten lines, then it scrolls) and collapses it again.
//!
//! [`BalloonStack`] is the model, without a toolkit clock (the caller
//! passes the time); [`render_stack`] draws it.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, BoxShadow, FontWeight, IntoElement, LineFragment, LineWrapper, Pixels,
    SharedString, Window, div, point, prelude::*, px,
};
use themes::UiRole;

use crate::components::Tooltip;
use crate::icons::{Icon, IconName};
use crate::pointer::PointerExt as _;
use crate::theme::ThemeExt;

/// How long an ordinary balloon stays (`NotificationsManagerImpl.notifyByBalloon`).
pub const BALLOON_TIME: Duration = Duration::from_secs(10);
/// How long a sticky balloon stays.
pub const STICKY_BALLOON_TIME: Duration = Duration::from_secs(300);
/// Balloons shown at once; more collapse into the strip
/// (`ActionCenterBalloonLayout.add`).
pub const VISIBLE: usize = 3;

/// Width of a balloon's body: the reference's fixed width (360 plus the
/// widths of the old shadow images, 10 each) less the New UI shadow
/// insets (`Notification.Shadow.borderInsets`, 5 on each side).
pub const WIDTH: f32 = 370.0;
/// Space between the stack and the right stripe and the status bar
/// (`ActionCenterBalloonLayout.setBounds`: 10 to the body).
pub const MARGIN: f32 = 10.0;
/// Space between two balloons: the shadow insets of both.
pub const GAP: f32 = 10.0;
/// Corner radius (`Notification.arc` 12 is a Java2D arc, a diameter).
const RADIUS: f32 = 6.0;
/// The shadow (`Notification.Shadow`: 5 px of `#00000010`).
const SHADOW: f32 = 5.0;
const SHADOW_ALPHA: f32 = 16.0 / 255.0;
/// `Notification.borderInsets` of the New UI (top, left, bottom; none on
/// the right) and the gap between the icon and the text.
const INSET_TOP: f32 = 4.0;
const INSET_LEFT: f32 = 4.0;
const INSET_BOTTOM: f32 = 6.0;
const ICON_GAP: f32 = 2.0;
/// Width of the icon column and the icon's distance from its left edge.
const ICON_COLUMN: f32 = 32.0;
const ICON_LEFT: f32 = 10.0;
const ICON: f32 = 16.0;
/// Room kept right of the title for the close button (5 + icon + 9).
const CLOSE_OFFSET: f32 = 5.0 + ICON + 9.0;
/// The close button: its icon 9 from the body's top and right edges, a
/// hover background 2 larger on each side (`CloseHoverBounds`,
/// `Button.arc` 8).
const CLOSE_EDGE: f32 = 9.0;
const CLOSE_HOVER_GAP: f32 = 2.0;
const CLOSE_RADIUS: f32 = 4.0;
/// Gap between action links (`BalloonLayoutConfiguration.actionGap`).
const ACTION_GAP: f32 = 16.0;
/// Content whose widest line is wider than this wraps at [`WRAP_WIDTH`]
/// (`BalloonLayoutConfiguration.MaxWidth`: the fixed width less 60).
pub const MAX_CONTENT_WIDTH: f32 = 320.0;
/// The width long content wraps at (`MaxWidthStyle`).
pub const WRAP_WIDTH: f32 = 240.0;
/// Lines an expanded balloon shows before its content scrolls
/// (`calculateContentHeight(10)`).
pub const MAX_EXPANDED_LINES: usize = 10;
/// The expand and collapse chevron's distance from the right edge
/// (`rightActionsOffset` of the New UI).
const EXPAND_RIGHT: f32 = 9.0;
/// Line height of the label font in balloons.
const LINE: f32 = 16.0;
/// The strip: 10 above its text and 4 below; its top 7 px lie under the
/// oldest balloon (`ActionCenterBalloonLayout.CollapseInfo`).
const MORE_TOP: f32 = 10.0;
const MORE_BOTTOM: f32 = 4.0;
const MORE_UNDER: f32 = 7.0;

/// Kind of a notification (`NotificationType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationKind {
    #[default]
    Information,
    Warning,
    Error,
}

impl NotificationKind {
    /// The kind named `information` (or `info`), `warning` or `error`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "information" | "info" => Some(NotificationKind::Information),
            "warning" => Some(NotificationKind::Warning),
            "error" => Some(NotificationKind::Error),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            NotificationKind::Information => "information",
            NotificationKind::Warning => "warning",
            NotificationKind::Error => "error",
        }
    }

    /// The balloon's icon (`NotificationsUtil.getIcon`, New UI mapping).
    pub fn icon(self) -> IconName {
        match self {
            NotificationKind::Information => IconName::INFO,
            NotificationKind::Warning => IconName::WARNING,
            NotificationKind::Error => IconName::ERROR,
        }
    }
}

/// An action link of a balloon: a command and its arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationAction {
    pub text: SharedString,
    pub command: String,
    pub args: serde_json::Value,
}

/// What a balloon shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notification {
    pub kind: NotificationKind,
    pub title: SharedString,
    pub content: SharedString,
    pub actions: Vec<NotificationAction>,
    /// Stays 5 min instead of 10 s.
    pub sticky: bool,
}

impl Notification {
    /// A notification of `kind`; errors are sticky.
    pub fn new(
        kind: NotificationKind,
        title: impl Into<SharedString>,
        content: impl Into<SharedString>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            content: content.into(),
            actions: Vec::new(),
            sticky: kind == NotificationKind::Error,
        }
    }

    pub fn with_action(
        mut self,
        text: impl Into<SharedString>,
        command: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        self.actions.push(NotificationAction {
            text: text.into(),
            command: command.into(),
            args,
        });
        self
    }

    fn time(&self) -> Duration {
        if self.sticky {
            STICKY_BALLOON_TIME
        } else {
            BALLOON_TIME
        }
    }
}

/// A shown balloon.
#[derive(Debug, Clone)]
pub struct Balloon {
    pub id: u64,
    pub notification: Notification,
    /// Time left before it goes.
    remaining: Duration,
    /// The time runs since then; `None` while it waits for input, the
    /// mouse is over it or the window is inactive.
    running_since: Option<Instant>,
    pub hovered: bool,
    /// The action link under the mouse (underlined).
    pub hovered_action: Option<usize>,
    /// Notifications collapsed into the strip below this balloon.
    pub collapsed: usize,
    /// The whole content is shown (the chevron was clicked).
    pub expanded: bool,
    /// The content's lines, wrapped by [`BalloonStack::wrap`] for the font
    /// they were wrapped with.
    lines: Vec<SharedString>,
    /// The width of the widest of them (the advances of its characters).
    content_width: f32,
    wrapped_for: Option<(SharedString, Pixels)>,
}

impl Balloon {
    /// The content's lines (wrapped by [`BalloonStack::wrap`]).
    pub fn lines(&self) -> &[SharedString] {
        &self.lines
    }

    /// Lines of content shown while collapsed: four less a title and an
    /// action row (`NotificationsManagerImpl.createBalloon`).
    pub fn line_limit(&self) -> usize {
        let n = &self.notification;
        4 - usize::from(!n.title.is_empty()) - usize::from(!n.actions.is_empty())
    }

    /// The content has more lines than the balloon shows collapsed.
    pub fn is_expandable(&self) -> bool {
        self.lines.len() > self.line_limit()
    }

    /// The lines shown now.
    pub fn shown_lines(&self) -> &[SharedString] {
        if self.is_expandable() && !self.expanded {
            &self.lines[..self.line_limit()]
        } else {
            &self.lines
        }
    }

    fn left(&self, now: Instant) -> Duration {
        match self.running_since {
            Some(since) => self
                .remaining
                .saturating_sub(now.saturating_duration_since(since)),
            None => self.remaining,
        }
    }

    fn pause(&mut self, now: Instant) {
        self.remaining = self.left(now);
        self.running_since = None;
    }
}

/// The balloons of a window, oldest first.
#[derive(Debug, Default)]
pub struct BalloonStack {
    balloons: Vec<Balloon>,
    next_id: u64,
    /// The window is active (the time runs only then).
    inactive: bool,
}

impl BalloonStack {
    pub fn balloons(&self) -> &[Balloon] {
        &self.balloons
    }

    pub fn is_empty(&self) -> bool {
        self.balloons.is_empty()
    }

    /// Notifications collapsed into the strip.
    pub fn collapsed(&self) -> usize {
        self.balloons.first().map_or(0, |b| b.collapsed)
    }

    /// Shows a balloon on top; with three shown, the oldest collapses into
    /// the strip, which moves to the next one. Returns its id.
    pub fn add(&mut self, notification: Notification) -> u64 {
        if self.balloons.len() >= VISIBLE {
            let oldest = self.balloons.remove(0);
            self.balloons[0].collapsed = oldest.collapsed + 1;
        }
        self.next_id += 1;
        let remaining = notification.time();
        self.balloons.push(Balloon {
            id: self.next_id,
            notification,
            remaining,
            running_since: None,
            hovered: false,
            hovered_action: None,
            collapsed: 0,
            expanded: false,
            lines: Vec::new(),
            content_width: 0.0,
            wrapped_for: None,
        });
        self.next_id
    }

    /// Wraps the content of the balloons for the UI font (before they are
    /// drawn; again when the font changed).
    pub fn wrap(&mut self, cx: &App) {
        let theme = cx.theme();
        let key = (theme.ui_family(), theme.ui_size());
        if self
            .balloons
            .iter()
            .all(|b| b.wrapped_for.as_ref() == Some(&key))
        {
            return;
        }
        let font = gpui::font(key.0.clone());
        let text_system = cx.text_system();
        let font_id = text_system.resolve_font(&font);
        let width = |line: &str| -> f32 {
            line.chars()
                .filter_map(|c| text_system.advance(font_id, key.1, c).ok())
                .map(|a| f32::from(a.width))
                .sum()
        };
        let mut wrapper = text_system.line_wrapper(font, key.1);
        for b in &mut self.balloons {
            if b.wrapped_for.as_ref() != Some(&key) {
                b.lines = content_lines(&b.notification.content, &mut wrapper);
                b.content_width = b.lines.iter().map(|l| width(l)).fold(0.0, f32::max);
                b.wrapped_for = Some(key.clone());
            }
        }
    }

    /// Expands a balloon to its whole content or collapses it again.
    pub fn toggle_expanded(&mut self, id: u64) {
        if let Some(b) = self.balloons.iter_mut().find(|b| b.id == id) {
            b.expanded = !b.expanded;
        }
    }

    /// Closes a balloon (its strip goes with it). Returns whether it was
    /// shown.
    pub fn close(&mut self, id: u64) -> bool {
        let before = self.balloons.len();
        self.balloons.retain(|b| b.id != id);
        self.balloons.len() != before
    }

    /// Closes every balloon (Alt-click on a close button, the strip).
    pub fn close_all(&mut self) {
        self.balloons.clear();
    }

    pub fn get(&self, id: u64) -> Option<&Balloon> {
        self.balloons.iter().find(|b| b.id == id)
    }

    /// Keyboard or mouse input: waiting balloons start their time (the
    /// reference's smart fade-out starts at the first input event).
    pub fn input(&mut self, now: Instant) {
        if self.inactive {
            return;
        }
        for b in &mut self.balloons {
            if b.running_since.is_none() && !b.hovered {
                b.running_since = Some(now);
            }
        }
    }

    /// The window became active or inactive: inactive, the time stops
    /// and starts again at the next input.
    pub fn set_active(&mut self, active: bool, now: Instant) {
        self.inactive = !active;
        if !active {
            for b in &mut self.balloons {
                b.pause(now);
            }
        }
    }

    /// The mouse entered or left a balloon: its time stops while it is
    /// over it.
    pub fn set_hovered(&mut self, id: u64, hovered: bool, now: Instant) {
        let inactive = self.inactive;
        if let Some(b) = self.balloons.iter_mut().find(|b| b.id == id) {
            b.hovered = hovered;
            if hovered {
                b.pause(now);
            } else {
                b.hovered_action = None;
                if !inactive {
                    b.running_since = Some(now);
                }
            }
        }
    }

    pub fn set_hovered_action(&mut self, id: u64, action: Option<usize>) {
        if let Some(b) = self.balloons.iter_mut().find(|b| b.id == id) {
            b.hovered_action = action;
        }
    }

    /// Closes the balloons whose time is over; returns whether any went.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.balloons.len();
        self.balloons
            .retain(|b| b.running_since.is_none() || !b.left(now).is_zero());
        self.balloons.len() != before
    }

    /// When the next balloon goes, if one's time runs.
    pub fn next_expiry(&self, now: Instant) -> Option<Duration> {
        self.balloons
            .iter()
            .filter(|b| b.running_since.is_some())
            .map(|b| b.left(now))
            .min()
    }

    /// The balloons as JSON, newest first (for `workbench.state`).
    pub fn state_json(&self, now: Instant) -> serde_json::Value {
        let balloons: Vec<_> = self
            .balloons
            .iter()
            .rev()
            .map(|b| {
                serde_json::json!({
                    "id": b.id,
                    "kind": b.notification.kind.name(),
                    "title": b.notification.title.as_ref(),
                    "content": b.notification.content.as_ref(),
                    "actions": b.notification.actions.iter()
                        .map(|a| a.text.as_ref()).collect::<Vec<_>>(),
                    "sticky": b.notification.sticky,
                    "lines": b.lines.iter().map(|l| l.as_ref()).collect::<Vec<_>>(),
                    "content_width": b.content_width,
                    "shown_lines": b.shown_lines().len(),
                    "expandable": b.is_expandable(),
                    "expanded": b.expanded,
                    "hovered": b.hovered,
                    "running": b.running_since.is_some(),
                    "remaining_ms": b.left(now).as_millis() as u64,
                })
            })
            .collect();
        serde_json::json!({ "balloons": balloons, "collapsed": self.collapsed() })
    }
}

/// What the mouse did on the balloons.
#[derive(Debug, Clone, PartialEq)]
pub enum BalloonEvent {
    /// The close button; with Alt every balloon closes.
    Close {
        id: u64,
        all: bool,
    },
    /// An action link.
    Action {
        id: u64,
        index: usize,
    },
    Hover {
        id: u64,
        hovered: bool,
    },
    HoverAction {
        id: u64,
        index: Option<usize>,
    },
    /// The chevron, or the collapsed content: expands or collapses.
    Expand {
        id: u64,
    },
    /// The "N more notifications" strip.
    ShowMore,
}

pub type BalloonHandler = Rc<dyn Fn(&BalloonEvent, &mut Window, &mut App)>;

/// "{0} more {1}" (`notifications.collapse.balloon.title`).
pub fn more_text(count: usize) -> String {
    let noun = if count == 1 {
        "notification"
    } else {
        "notifications"
    };
    format!("{count} more {noun}")
}

/// Tooltip of the close button (`tooltip.close.notification`).
pub fn close_tooltip() -> &'static str {
    if cfg!(target_os = "macos") {
        "Close. ⌥click to close all notifications"
    } else {
        "Close. Alt+click to close all notifications"
    }
}

/// The lines of a balloon's content: its lines as they are while the
/// widest fits in [`MAX_CONTENT_WIDTH`], else wrapped at [`WRAP_WIDTH`]
/// (`NotificationsManagerImpl.createBalloon`: the content gets the
/// `width:240px` style when its preferred width is over `MaxWidth`).
pub fn content_lines(text: &str, wrapper: &mut LineWrapper) -> Vec<SharedString> {
    if text.is_empty() {
        return Vec::new();
    }
    let wraps_at = |wrapper: &mut LineWrapper, line: &str, width: f32| -> Vec<usize> {
        let fragments = [LineFragment::text(line)];
        wrapper
            .wrap_line(&fragments, px(width))
            .map(|b| b.ix)
            .collect()
    };
    let wide = text
        .lines()
        .any(|line| !wraps_at(wrapper, line, MAX_CONTENT_WIDTH).is_empty());
    let mut out = Vec::new();
    for line in text.lines() {
        let breaks = if wide {
            wraps_at(wrapper, line, WRAP_WIDTH)
        } else {
            Vec::new()
        };
        let mut start = 0;
        for end in breaks.into_iter().chain([line.len()]) {
            out.push(SharedString::from(line[start..end].trim_end().to_owned()));
            start = end;
        }
    }
    out
}

/// Vertical spaces of a balloon (`BalloonLayoutConfiguration`): top,
/// title to content, content to actions, title to actions, bottom, and
/// the icon's distance from the top.
struct Spaces {
    top: f32,
    title_content: f32,
    content_actions: f32,
    title_actions: f32,
    bottom: f32,
    icon_top: f32,
}

const TWO_LINES: Spaces = Spaces {
    top: 11.0,
    title_content: 5.0,
    content_actions: 5.0,
    title_actions: 5.0,
    bottom: 14.0,
    icon_top: 11.0,
};
const THREE_LINES: Spaces = Spaces {
    top: 7.0,
    title_content: 3.0,
    content_actions: 7.0,
    title_actions: 0.0,
    bottom: 8.0,
    icon_top: 7.0,
};

fn render_balloon(b: &Balloon, on: &BalloonHandler, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let n = &b.notification;
    let id = b.id;
    let has_title = !n.title.is_empty();
    let has_content = !n.content.is_empty();
    let has_actions = !n.actions.is_empty();
    // `BalloonLayoutConfiguration.create`: three lines with a title,
    // content and actions, or with content of more than one line (two
    // without a title and actions).
    let one_line = if has_title || has_actions { 1 } else { 2 };
    let spaces =
        if (has_title && has_content && has_actions) || (has_content && b.lines.len() > one_line) {
            &THREE_LINES
        } else {
            &TWO_LINES
        };
    let expandable = b.is_expandable();
    let text = theme.color(UiRole::NotificationText);
    let link = theme.color(UiRole::NotificationLink);

    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .pt(px(spaces.top))
        .pb(px(spaces.bottom))
        .line_height(px(LINE));
    if has_title {
        column = column.child(
            div()
                .debug_selector(move || format!("balloon-title-{id}"))
                .pr(px(CLOSE_OFFSET))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .when(has_content, |d| d.font_weight(FontWeight::BOLD))
                .child(n.title.clone()),
        );
    }
    if has_content {
        if has_title {
            column = column.child(div().h(px(spaces.title_content)));
        }
        column = column.child(render_content(b, on));
    }
    if has_actions {
        let space = if has_content {
            spaces.content_actions
        } else if has_title {
            spaces.title_actions
        } else {
            0.0
        };
        let mut row = div().flex().flex_row().gap(px(ACTION_GAP)).mt(px(space));
        for (index, action) in n.actions.iter().enumerate() {
            let hover = on.clone();
            let click = on.clone();
            let underline = b.hovered_action == Some(index);
            row = row.child(
                div()
                    .id(("balloon-action", id as usize * 64 + index))
                    .debug_selector(move || format!("balloon-action-{id}-{index}"))
                    .relative()
                    .whitespace_nowrap()
                    .text_color(link)
                    .when(underline, |d| {
                        d.text_decoration_1().text_decoration_color(link)
                    })
                    .on_hover(move |hovered, window, cx| {
                        let index = hovered.then_some(index);
                        hover(&BalloonEvent::HoverAction { id, index }, window, cx)
                    })
                    .on_click(move |_, window, cx| {
                        click(&BalloonEvent::Action { id, index }, window, cx)
                    })
                    .child(action.text.clone())
                    .pointing_hand(),
            );
        }
        if expandable && b.expanded {
            row = row.child(div().flex_1()).child(chevron(b, on));
        }
        column = column.child(row);
    } else if expandable && b.expanded {
        // The chevron takes the action row's place (`CenteredLayoutWithActions`).
        column = column.child(div().flex().flex_row().justify_end().child(chevron(b, on)));
    }

    let close = {
        let on = on.clone();
        div()
            .id(("balloon-close", id as usize))
            .debug_selector(move || format!("balloon-close-{id}"))
            .absolute()
            .top(px(CLOSE_EDGE - CLOSE_HOVER_GAP))
            .right(px(CLOSE_EDGE - CLOSE_HOVER_GAP))
            .p(px(CLOSE_HOVER_GAP))
            .rounded(px(CLOSE_RADIUS))
            .hover(|s| s.bg(theme.color(UiRole::NotificationIconHover)))
            .tooltip(Tooltip::text(close_tooltip(), None))
            .on_click(move |e, window, cx| {
                let all = e.modifiers().alt;
                on(&BalloonEvent::Close { id, all }, window, cx)
            })
            .child(Icon::new(IconName::CLOSE).size(px(ICON)))
    };

    let hover = on.clone();
    div()
        .id(("balloon", id as usize))
        .debug_selector(move || format!("balloon-{id}"))
        .occlude()
        .relative()
        .flex()
        .flex_row()
        .flex_none()
        .w(px(WIDTH))
        .pt(px(INSET_TOP))
        .pl(px(INSET_LEFT))
        .pb(px(INSET_BOTTOM))
        .gap(px(ICON_GAP))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(theme.color(UiRole::NotificationBorder))
        .bg(theme.color(UiRole::Notification))
        .shadow(shadow())
        .text_color(text)
        .on_hover(move |hovered, window, cx| {
            hover(
                &BalloonEvent::Hover {
                    id,
                    hovered: *hovered,
                },
                window,
                cx,
            )
        })
        .child(
            div()
                .flex_none()
                .w(px(ICON_COLUMN))
                .pl(px(ICON_LEFT))
                .pt(px(spaces.icon_top))
                .child(Icon::new(n.kind.icon()).size(px(ICON))),
        )
        .child(column)
        // Shown while the mouse is over the balloon (`ActionButton.hasPaint`).
        .when(b.hovered, |d| d.child(close))
        .into_any_element()
}

/// The content's lines: collapsed, the first [`Balloon::line_limit`]
/// with the expand chevron at the right of the last one, and a click
/// anywhere on them expands (`HoverAdapter`); expanded, all of them, up to
/// [`MAX_EXPANDED_LINES`] before they scroll.
fn render_content(b: &Balloon, on: &BalloonHandler) -> AnyElement {
    let id = b.id;
    let shown = b.shown_lines();
    let lines = shown.iter().enumerate().map(|(i, line)| {
        div()
            .debug_selector(move || format!("balloon-line-{id}-{i}"))
            .h(px(LINE))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(line.clone())
            .into_any_element()
    });
    let content = div()
        .id(("balloon-content", id as usize))
        .debug_selector(move || format!("balloon-content-{id}"))
        .relative()
        .flex()
        .flex_col()
        .children(lines);
    if !b.is_expandable() {
        return content.into_any_element();
    }
    if b.expanded {
        if shown.len() <= MAX_EXPANDED_LINES {
            return content.into_any_element();
        }
        return content
            .h(px(LINE * MAX_EXPANDED_LINES as f32))
            .overflow_y_scroll()
            .into_any_element();
    }
    let last = (shown.len().max(1) - 1) as f32;
    let on = on.clone();
    content
        .child(
            div()
                .absolute()
                .top(px(LINE * last))
                .right(px(EXPAND_RIGHT))
                .child(chevron(b, &on)),
        )
        .on_click(move |_, window, cx| on(&BalloonEvent::Expand { id }, window, cx))
        .pointing_hand()
        .into_any_element()
}

/// The expand chevron (down) or, expanded, the collapse one (up)
/// (`AllIcons.Ide.Notification.Expand` and `Collapse`, New UI
/// `chevronDown` and `chevronUp`).
fn chevron(b: &Balloon, on: &BalloonHandler) -> AnyElement {
    let id = b.id;
    let icon = if b.expanded {
        IconName::CHEVRON_UP
    } else {
        IconName::CHEVRON_DOWN
    };
    let on = on.clone();
    div()
        .id(("balloon-expand", id as usize))
        .debug_selector(move || format!("balloon-expand-{id}"))
        .flex_none()
        .size(px(ICON))
        .child(Icon::new(icon).size(px(ICON)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on(&BalloonEvent::Expand { id }, window, cx)
        })
        .pointing_hand()
        .into_any_element()
}

fn shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui::hsla(0.0, 0.0, 0.0, SHADOW_ALPHA),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(SHADOW),
        spread_radius: px(0.0),
    }]
}

/// The strip below the oldest balloon that counts the collapsed ones.
fn render_more(count: usize, on: &BalloonHandler, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let on = on.clone();
    div()
        .id("balloon-more")
        .debug_selector(|| "balloon-more".into())
        .occlude()
        .relative()
        .flex()
        .justify_center()
        .w(px(WIDTH))
        .mt(px(-MORE_UNDER))
        .pt(px(MORE_TOP))
        .pb(px(MORE_BOTTOM))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(theme.color(UiRole::NotificationBorder))
        .bg(theme.color(UiRole::NotificationMore))
        .shadow(shadow())
        .text_size(theme.ui_size() - px(1.0))
        .line_height(px(LINE))
        .text_color(theme.color(UiRole::NotificationMoreText))
        .on_click(move |_, window, cx| on(&BalloonEvent::ShowMore, window, cx))
        .child(more_text(count))
        .pointing_hand()
        .into_any_element()
}

/// The balloons stacked upwards from the bottom right: the caller places
/// the returned element (absolute) with [`MARGIN`] to the right stripe
/// and the status bar. The balloons' content is wrapped first
/// ([`BalloonStack::wrap`]).
pub fn render_stack(stack: &BalloonStack, on: BalloonHandler, cx: &App) -> Option<AnyElement> {
    if stack.is_empty() {
        return None;
    }
    // Column from the bottom: the strip first (painted under the oldest
    // balloon), then the balloons from the oldest up.
    let mut column = div()
        .flex()
        .flex_col_reverse()
        .items_end()
        .font_family(cx.theme().ui_family())
        .text_size(cx.theme().ui_size());
    let collapsed = stack.collapsed();
    if collapsed > 0 {
        column = column.child(render_more(collapsed, &on, cx));
    }
    for (i, b) in stack.balloons().iter().enumerate() {
        let balloon = render_balloon(b, &on, cx);
        let space = if i == 0 { 0.0 } else { GAP };
        column = column.child(div().mb(px(space)).child(balloon));
    }
    Some(column.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str) -> Notification {
        Notification::new(NotificationKind::Information, title.to_owned(), "")
    }

    #[test]
    fn three_are_shown_and_older_ones_collapse() {
        let mut s = BalloonStack::default();
        let a = s.add(info("a"));
        s.add(info("b"));
        s.add(info("c"));
        assert_eq!(s.collapsed(), 0);
        s.add(info("d"));
        let titles: Vec<_> = s
            .balloons()
            .iter()
            .map(|b| b.notification.title.as_ref())
            .collect();
        assert_eq!(titles, ["b", "c", "d"]);
        assert_eq!(s.collapsed(), 1);
        assert!(s.get(a).is_none());
        s.add(info("e"));
        assert_eq!(s.collapsed(), 2);
        assert_eq!(more_text(2), "2 more notifications");
        assert_eq!(more_text(1), "1 more notification");
        // Closing the oldest takes the strip with it; others keep it.
        let newest = s.balloons()[2].id;
        s.close(newest);
        assert_eq!(s.collapsed(), 2);
        let oldest = s.balloons()[0].id;
        s.close(oldest);
        assert_eq!(s.collapsed(), 0);
    }

    #[test]
    fn time_starts_at_input_and_stops_while_hovered_or_inactive() {
        let t0 = Instant::now();
        let mut s = BalloonStack::default();
        let id = s.add(info("a"));
        let err = s.add(Notification::new(NotificationKind::Error, "e", "x"));
        assert!(s.get(err).unwrap().notification.sticky);
        // Waiting for input: nothing goes.
        assert!(!s.expire(t0 + Duration::from_secs(60)));
        assert_eq!(s.next_expiry(t0), None);
        s.input(t0);
        assert_eq!(s.next_expiry(t0), Some(BALLOON_TIME));
        // Hovered for 20 s after 4 s: 6 s left afterwards.
        let t4 = t0 + Duration::from_secs(4);
        s.set_hovered(id, true, t4);
        assert!(!s.expire(t4 + Duration::from_secs(20)));
        let t24 = t4 + Duration::from_secs(20);
        s.set_hovered(id, false, t24);
        assert_eq!(s.next_expiry(t24), Some(Duration::from_secs(6)));
        // Inactive window: stops, starts again at the next input.
        let t25 = t24 + Duration::from_secs(1);
        s.set_active(false, t25);
        s.input(t25 + Duration::from_secs(100));
        assert!(!s.expire(t25 + Duration::from_secs(100)));
        s.set_active(true, t25);
        let t30 = t25 + Duration::from_secs(5);
        s.input(t30);
        assert!(s.expire(t30 + Duration::from_secs(5)));
        assert!(s.get(id).is_none());
        // The error stays for 5 min.
        assert!(s.get(err).is_some());
        assert!(s.expire(t30 + STICKY_BALLOON_TIME));
        assert!(s.is_empty());
    }
}
