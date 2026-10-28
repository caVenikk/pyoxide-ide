//! Links in the session view: which links the screen shows, the one under
//! the pointer, and what a click does with it (the reference's
//! `InvisibleHyperlinkHintManager` and hyperlink handling in the output
//! editor, IntelliJ Community, Apache-2.0).
//!
//! - A visible link (URL, OSC 8, a product filter's) is underlined all the
//!   time, has the hand cursor under the pointer, and a click opens it.
//! - A path in the text is a link only while Cmd (Ctrl) is held over it:
//!   then it is underlined, has the hand cursor, and Cmd/Ctrl+click opens
//!   it. A plain click on it shows a hint, "Open" and the shortcut, above
//!   the click once the double-click time has passed.
//!
//! What the filters find is looked up in a cache. The product's filters
//! and the check that a path exists read the file system, so they run on
//! the background executor and the screen catches up when they are done.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Context, Corner, IntoElement, Modifiers, MouseDownEvent, MouseUpEvent, Pixels,
    Point as GpuiPoint, SharedString, Task, anchored, div, point, prelude::*, px,
};
use terminal::Point;
use themes::{Metric, UiRole};
use ui::deferred;
use ui::theme::ThemeExt;

use crate::links::{
    self, GridLink, Link, LinkFilters, LinkTarget, PathToken, logical_lines, path_token_at, scan,
};
use crate::view::{SessionView, SessionViewEvent};

/// How long after a press the hint of a path may show: a second click
/// within it is a double click (`editor.invisible.hyperlink.popup.delay.ms`).
pub const HINT_DELAY: Duration = Duration::from_millis(180);
/// The most lines whose filter results are kept.
const CACHED_LINES: usize = 4096;
/// The hint's size, for telling when the pointer left it.
const HINT_REACH: (f32, f32) = (140.0, 44.0);

/// The links of a view.
#[derive(Default)]
pub(crate) struct LinkState {
    pub filters: Option<LinkFilters>,
    /// What the product's filters found per line of output.
    filtered: HashMap<String, Vec<Link>>,
    /// Lines being filtered off the UI thread.
    filtering: HashSet<String>,
    filter_task: Option<Task<()>>,
    /// The snapshot the visible links belong to.
    scanned: Option<u64>,
    /// The visible links of the screen.
    pub grid: Vec<GridLink>,
    /// Cmd (Ctrl) is held.
    pub command: bool,
    /// The pointer, over the view.
    pub pointer: Option<GpuiPoint<Pixels>>,
    /// The link under the pointer: a visible one, or a path while Cmd
    /// (Ctrl) is held.
    pub hover: Option<GridLink>,
    /// Whether a path word names a file or folder, by working directory
    /// and word.
    paths: HashMap<(Option<String>, String), Option<LinkTarget>>,
    resolving: Option<Task<()>>,
    /// The last press: where, when, and whether it began a click.
    press: Option<Press>,
    /// Open the link at this cell once its path is checked.
    open_when_resolved: Option<Point>,
    /// A hint to show once the delay has passed and the path is checked.
    hint_wanted: Option<HintWanted>,
    hint_task: Option<Task<()>>,
    pub hint: Option<Hint>,
}

#[derive(Clone, Copy)]
struct Press {
    cell: Point,
    at: Instant,
    /// The press began a plain single click: no selection was there to
    /// lose, no modifier but Cmd (Ctrl).
    click: bool,
    command: bool,
}

#[derive(Clone, Copy)]
struct HintWanted {
    cell: Point,
    position: GpuiPoint<Pixels>,
    due: Instant,
}

/// The hint "Open ⌘Click" over a path that was clicked.
#[derive(Clone, Debug)]
pub(crate) struct Hint {
    pub position: GpuiPoint<Pixels>,
    pub link: GridLink,
}

/// The answer to what is at a cell.
enum Lookup {
    Link(GridLink),
    /// A path word whose file is being checked.
    Pending,
    Nothing,
}

/// Cmd on macOS, Ctrl elsewhere (`InputEvent.META_DOWN_MASK` or
/// `CTRL_DOWN_MASK`).
pub fn command_held(modifiers: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.platform
    } else {
        modifiers.control
    }
}

/// The shortcut shown in the hint.
fn click_text() -> &'static str {
    if cfg!(target_os = "macos") {
        "\u{2318}Click"
    } else {
        "Ctrl+Click"
    }
}

impl SessionView {
    /// What a product adds to the links: its filters, for a shell working
    /// in a folder.
    pub fn set_link_filters(&mut self, filters: Option<LinkFilters>, cx: &mut Context<Self>) {
        self.links.filters = filters;
        self.links.filtered.clear();
        self.links.filtering.clear();
        self.links.filter_task = None;
        self.links.scanned = None;
        cx.notify();
    }

    /// The visible links of the screen drawn last.
    pub fn links(&self) -> &[GridLink] {
        &self.links.grid
    }

    /// The link under the pointer.
    pub fn hovered_link(&self) -> Option<&GridLink> {
        self.links.hover.as_ref()
    }

    /// Looks for the links of a new snapshot.
    pub(crate) fn refresh_links(&mut self, cx: &mut Context<Self>) {
        if self.links.scanned == Some(self.snapshots) {
            return;
        }
        self.links.scanned = Some(self.snapshots);
        let lines = logical_lines(&self.snapshot);
        let result = scan(
            &self.snapshot,
            &lines,
            &self.links.filtered,
            self.links.filters.is_some(),
        );
        self.links.grid = result.links;
        if !result.unfiltered.is_empty() {
            self.filter_lines(result.unfiltered, cx);
        }
        self.update_hover(cx);
    }

    /// Runs the product's filters over lines, off the UI thread.
    fn filter_lines(&mut self, mut lines: Vec<String>, cx: &mut Context<Self>) {
        let Some(filters) = self.links.filters.clone() else {
            return;
        };
        if self.links.filter_task.is_some() {
            // The finished task scans again.
            return;
        }
        lines.retain(|l| !self.links.filtering.contains(l));
        lines.dedup();
        if lines.is_empty() {
            return;
        }
        self.links.filtering.extend(lines.iter().cloned());
        let filters = filters(self.working_directory().cloned());
        let work = cx.background_spawn(async move {
            lines
                .into_iter()
                .map(|line| {
                    let mut found = Vec::new();
                    for filter in &filters {
                        filter.links(&line, &mut found);
                    }
                    (line, found)
                })
                .collect::<Vec<_>>()
        });
        self.links.filter_task = Some(cx.spawn(async move |this, cx| {
            let results = work.await;
            let _ = this.update(cx, |v, cx| {
                if v.links.filtered.len() > CACHED_LINES {
                    v.links.filtered.clear();
                }
                v.links.filtering.clear();
                v.links.filter_task = None;
                v.links.filtered.extend(results);
                v.links.scanned = None;
                cx.notify();
            });
        }));
    }

    /// The cells to underline: the visible links, and a path under the
    /// pointer while Cmd (Ctrl) is held.
    pub(crate) fn link_underlines(&self) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        for link in self.links.grid.iter().filter(|l| l.visible) {
            out.extend(link.segments.iter().cloned());
        }
        if let Some(hover) = self.links.hover.as_ref().filter(|h| !h.visible) {
            out.extend(hover.segments.iter().cloned());
        }
        out
    }

    /// The pointer is over a link.
    pub(crate) fn link_pointer(&self) -> bool {
        self.links.hover.is_some()
    }

    // ---- hover --------------------------------------------------------------

    /// The pointer moved over the view (or left it: `None`).
    pub(crate) fn pointer_moved(
        &mut self,
        position: Option<GpuiPoint<Pixels>>,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.links.pointer = position;
        self.links.command = command_held(modifiers);
        self.update_hover(cx);
        self.hint_follows_pointer(cx);
    }

    /// Cmd (Ctrl) was pressed or released.
    pub(crate) fn modifiers_changed(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        let command = command_held(modifiers);
        if command != self.links.command {
            self.links.command = command;
            self.update_hover(cx);
        }
    }

    /// The link under the pointer, for the underline and the cursor.
    fn update_hover(&mut self, cx: &mut Context<Self>) {
        let cell = self.pointer_cell();
        // The program has the mouse (Shift takes it back): no links.
        let program_has_mouse =
            self.options.mouse_reporting && self.snapshot.modes.mouse != terminal::MouseMode::None;
        let hover = match cell {
            Some(_) if program_has_mouse => None,
            Some(cell) => {
                let visible = self
                    .links
                    .grid
                    .iter()
                    .find(|l| l.visible && l.contains(cell))
                    .cloned();
                match visible {
                    Some(link) => Some(link),
                    None if self.links.command => match self.lookup(cell, cx) {
                        Lookup::Link(link) => Some(link),
                        Lookup::Pending | Lookup::Nothing => None,
                    },
                    None => None,
                }
            }
            None => None,
        };
        if hover != self.links.hover {
            self.links.hover = hover;
            cx.notify();
        }
    }

    /// The cell under the pointer, when it is over the grid.
    fn pointer_cell(&self) -> Option<Point> {
        let position = self.links.pointer?;
        let geometry = self.geometry.as_ref()?;
        if !geometry.bounds.contains(&position) {
            return None;
        }
        let (cell, _) = geometry.cell_at(position);
        let c = self
            .snapshot
            .cells
            .get(cell.line * self.snapshot.columns + cell.column)?;
        (c.c != ' ' && c.c != '\0').then_some(cell)
    }

    /// What is at a cell: a visible link, or a path word whose file exists.
    /// A path is checked off the UI thread; until it is, `Pending`.
    fn lookup(&mut self, cell: Point, cx: &mut Context<Self>) -> Lookup {
        if let Some(link) = self
            .links
            .grid
            .iter()
            .find(|l| l.visible && l.contains(cell))
        {
            return Lookup::Link(link.clone());
        }
        let lines = logical_lines(&self.snapshot);
        let Some((line, token)) = lines
            .iter()
            .find_map(|line| path_token_at(line, cell).map(|t| (line, t)))
        else {
            return Lookup::Nothing;
        };
        let cwd = self.working_directory().map(|p| p.display().to_string());
        let key = (cwd, line.text[token.range.clone()].to_owned());
        match self.links.paths.get(&key) {
            Some(Some(target)) => Lookup::Link(GridLink {
                segments: line.segments(token.range.clone()),
                target: target.clone(),
                visible: false,
            }),
            Some(None) => Lookup::Nothing,
            None => {
                self.resolve_path(key, token, cx);
                Lookup::Pending
            }
        }
    }

    /// Checks a path word against the file system, off the UI thread.
    fn resolve_path(
        &mut self,
        key: (Option<String>, String),
        token: PathToken,
        cx: &mut Context<Self>,
    ) {
        if self.links.resolving.is_some() {
            return;
        }
        let cwd = self.working_directory().cloned();
        let work = cx.background_spawn(async move { links::file_link(&token, cwd.as_deref()) });
        self.links.resolving = Some(cx.spawn(async move |this, cx| {
            let target = work.await;
            let _ = this.update(cx, |v, cx| {
                if v.links.paths.len() > CACHED_LINES {
                    v.links.paths.clear();
                }
                v.links.resolving = None;
                v.links.paths.insert(key, target);
                v.path_resolved(cx);
            });
        }));
    }

    /// A path was checked: what waited for it goes on.
    fn path_resolved(&mut self, cx: &mut Context<Self>) {
        self.update_hover(cx);
        if let Some(cell) = self.links.open_when_resolved.take() {
            match self.lookup(cell, cx) {
                Lookup::Link(link) => self.open_target(link.target, cx),
                Lookup::Pending => self.links.open_when_resolved = Some(cell),
                Lookup::Nothing => {}
            }
        }
        self.show_hint_if_due(cx);
    }

    // ---- clicks -------------------------------------------------------------

    /// A press on the grid: a plain single click may open a link on its
    /// release or show the hint.
    pub(crate) fn link_pressed(
        &mut self,
        e: &MouseDownEvent,
        cell: Point,
        had_selection: bool,
        cx: &mut Context<Self>,
    ) {
        self.hide_hint(cx);
        self.links.hint_wanted = None;
        self.links.hint_task = None;
        let plain = !e.modifiers.shift && !e.modifiers.alt;
        let command = command_held(&e.modifiers);
        self.links.press = Some(Press {
            cell,
            at: Instant::now(),
            click: e.click_count == 1 && plain && !had_selection,
            command,
        });
    }

    /// The release that ends a click on a link or on a path.
    pub(crate) fn link_released(&mut self, e: &MouseUpEvent, cell: Point, cx: &mut Context<Self>) {
        let Some(press) = self.links.press.take() else {
            return;
        };
        if e.button != gpui::MouseButton::Left || press.cell != cell {
            return;
        }
        let visible = self
            .links
            .grid
            .iter()
            .find(|l| l.visible && l.contains(cell))
            .cloned();
        if let Some(link) = visible {
            if e.click_count == 1 {
                self.open_target(link.target, cx);
            }
            return;
        }
        if press.command {
            self.links.open_when_resolved = None;
            match self.lookup(cell, cx) {
                Lookup::Link(link) => self.open_target(link.target, cx),
                Lookup::Pending => self.links.open_when_resolved = Some(cell),
                Lookup::Nothing => {}
            }
            return;
        }
        if press.click {
            self.links.hint_wanted = Some(HintWanted {
                cell,
                position: e.position,
                due: press.at + HINT_DELAY,
            });
            let wait = HINT_DELAY.saturating_sub(press.at.elapsed());
            self.links.hint_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |v, cx| v.show_hint_if_due(cx));
            }));
            // Starts checking the path now.
            let _ = self.lookup(cell, cx);
        }
    }

    /// Opens the link at a cell of the screen (a click, or the
    /// `terminal.open-link` command); a path is checked first.
    pub fn open_link_at(&mut self, cell: Point, cx: &mut Context<Self>) {
        let cell = Point::new(
            cell.line.min(self.snapshot.rows.saturating_sub(1)),
            cell.column.min(self.snapshot.columns.saturating_sub(1)),
        );
        self.links.open_when_resolved = None;
        match self.lookup(cell, cx) {
            Lookup::Link(link) => self.open_target(link.target, cx),
            Lookup::Pending => self.links.open_when_resolved = Some(cell),
            Lookup::Nothing => {}
        }
    }

    fn open_target(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        self.hide_hint(cx);
        cx.emit(SessionViewEvent::OpenLink(target));
    }

    // ---- the hint -----------------------------------------------------------

    fn show_hint_if_due(&mut self, cx: &mut Context<Self>) {
        let Some(wanted) = self.links.hint_wanted else {
            return;
        };
        if Instant::now() < wanted.due {
            return;
        }
        match self.lookup(wanted.cell, cx) {
            Lookup::Link(link) => {
                self.links.hint_wanted = None;
                self.links.hint = Some(Hint {
                    position: wanted.position,
                    link,
                });
                cx.notify();
            }
            Lookup::Pending => {}
            Lookup::Nothing => self.links.hint_wanted = None,
        }
    }

    /// Hides the hint (any key, scrolling, a press, the pointer far away).
    pub(crate) fn hide_hint(&mut self, cx: &mut Context<Self>) {
        self.links.hint_wanted = None;
        self.links.hint_task = None;
        if self.links.hint.take().is_some() {
            cx.notify();
        }
    }

    /// The hint stays while the pointer is on the link, on the hint, or
    /// between them.
    fn hint_follows_pointer(&mut self, cx: &mut Context<Self>) {
        let (Some(hint), Some(pointer)) = (&self.links.hint, self.links.pointer) else {
            return;
        };
        let near = pointer.x >= hint.position.x - px(8.0)
            && pointer.x <= hint.position.x + px(HINT_REACH.0)
            && pointer.y >= hint.position.y - px(HINT_REACH.1)
            && pointer.y <= hint.position.y + px(4.0);
        let on_link = self
            .links
            .hover
            .as_ref()
            .is_some_and(|h| h.segments == hint.link.segments);
        let on_cell = self.geometry.as_ref().is_some_and(|g| {
            let (cell, _) = g.cell_at(pointer);
            hint.link.contains(cell)
        });
        if !(near || on_link || on_cell) {
            self.hide_hint(cx);
        }
    }

    /// The hint over the clicked path: "Open" as a link, then the click
    /// that opens (`InvisibleHyperlinkHintManager.getHtmlText`).
    pub(crate) fn render_hint(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hint = self.links.hint.as_ref()?;
        let theme = cx.theme();
        let target = hint.link.target.clone();
        let content = div()
            .id("terminal-link-hint")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .child(
                div()
                    .id("terminal-link-open")
                    .cursor_pointer()
                    .text_color(theme.color(UiRole::TextLink))
                    .child("Open")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_target(target.clone(), cx);
                    })),
            )
            .child(
                div()
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(SharedString::from(click_text())),
            );
        Some(
            deferred(
                anchored()
                    .position(point(hint.position.x, hint.position.y - px(6.0)))
                    .anchor(Corner::BottomLeft)
                    .snap_to_window_with_margin(px(4.0))
                    .child(content),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    /// The hint over a clicked path as JSON (`terminal.state`): the link it
    /// opens, or null.
    pub(crate) fn hint_json(&self) -> serde_json::Value {
        match &self.links.hint {
            Some(h) => serde_json::json!({ "target": h.link.describe() }),
            None => serde_json::Value::Null,
        }
    }

    /// The links of the visible lines as JSON (`terminal.state`).
    pub(crate) fn links_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.links
                .grid
                .iter()
                .map(|l| {
                    serde_json::json!({
                        "line": l.segments.first().map(|s| s.0),
                        "columns": l.segments.first().map(|s| [s.1.start, s.1.end]),
                        "segments": l.segments.iter().map(|(line, c)| [*line, c.start, c.end]).collect::<Vec<_>>(),
                        "kind": match l.target {
                            LinkTarget::Url(_) => "url",
                            LinkTarget::File { .. } => "file",
                        },
                        "target": l.describe(),
                        "visible": l.visible,
                    })
                })
                .collect(),
        )
    }
}
