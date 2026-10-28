//! The background tasks of a main window (`progress`): the status bar
//! widget and the Processes popup, as the reference's
//! `InfoAndProgressPanel` and `ProcessPopup` show them.
//!
//! The widget sits at the east end of the status bar's centre, before the
//! widgets: the text of the first task (its title while it has none, its
//! paused text while paused), its progress bar, its pause and cancel
//! buttons, and `+N` for the other tasks. A task appears once it has run
//! for [`progress::SHOW_DELAY`]. A click on the widget opens the Processes
//! popup at the bottom right of the window, listing every task with its
//! title, bar, buttons, text and details; while it is open the widget
//! reads "Hide processes (N)". The popup stays open on a click outside,
//! closes with Escape or its Hide button, and closes by itself when the
//! tasks it was opened for end. It is moved by its header and resized from
//! its edges ([`place`]); the window keeps that place while it is open.
//! With Auto Show (Window > Processes) it opens, without the focus, when a
//! second task shows in the active window
//! (`ide.windowSystem.autoShowProcessPopup`).
//!
//! Only the tasks of the window's project (and those of no project) are
//! shown. Updates arrive at most every [`progress::UPDATE_INTERVAL`]; an
//! indeterminate bar moves one frame every [`progress_bar::FRAME`] while
//! it is shown.

mod place;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point,
    Render, SharedString, Size, StatefulInteractiveElement, Styled, Task, Window, canvas, div,
    prelude::FluentBuilder, px, size,
};
use progress::{TaskId, TaskSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, popup_shadow};
use ui::icons::{Icon, IconName};
use ui::progress_bar::{self, BarState};
use ui::theme::ThemeExt;

use crate::popup::{self, Edges, HeldDrag, track_held_drag};
use place::{Drag, Margins, Place};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Show or hide the Processes popup.
    ShowProcesses => "background-tasks.show", "Show Processes",
        "Shows the Processes popup that lists the background tasks of the window, or hides it when it is open (`ShowProcessWindow`).";
    /// Cancel a background task.
    CancelBackgroundTask { id: u64 } => "background-tasks.cancel", "Cancel Background Task",
        "Cancels a background task, as its cancel button does: `id` as `workbench.state` lists it under `background_tasks`. The task shows \"Stopping\" until its work ends.",
        r#"{"id": 1}"#;
    /// Pause or resume a background task.
    PauseBackgroundTask { id: u64 } => "background-tasks.pause", "Pause or Resume Background Task",
        "Pauses a pausable background task, or resumes it when it is paused, as its pause button does: `id` as `workbench.state` lists it under `background_tasks`.",
        r#"{"id": 1}"#;
    /// Toggle Auto Show of the Processes popup.
    ToggleAutoShowProcesses => "background-tasks.toggle-auto-show", "Auto Show Processes",
        "Turns Auto Show on or off: the Processes popup opens by itself, without the focus, when a second background task shows in the active window (`AutoShowProcessWindow`, off by default).";
    /// Start a sample background task.
    StartBackgroundTask {
        title: String,
        text: Option<String>,
        details: Option<String>,
        fraction: Option<f64>,
        seconds: Option<f64>,
        cancellable: Option<bool>,
        paused_text: Option<String>,
    } => "background-tasks.start", "Start Background Task",
        "Starts a background task of the window's project that does nothing for `seconds` (10 by default) or until it is cancelled: a title, an optional text and details, a fraction from 0 to 1 (indeterminate without one), a cancel button unless `cancellable` is false and, with `paused_text`, a pause button (the text shows while it is paused). For agent runs and screenshots of the status bar and the Processes popup.",
        r#"{"title": "Indexing", "text": "Scanning files…", "fraction": 0.4, "seconds": 30}"#;
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "ProcessesPopup";
    /// Hide the Processes popup.
    HideProcesses => "background-tasks.hide", "Hide Processes",
        "Hides the Processes popup (Escape in it, its Hide button).";
}

pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
    settings_ui::store::register::<ProcessesSettings>(cx);
}

/// The options of the Processes popup (`[window.processes]`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProcessesSettings {
    /// The popup opens when a second task shows (the reference's registry
    /// key `ide.windowSystem.autoShowProcessPopup`, off by default).
    pub auto_show: bool,
}

impl settings::Section for ProcessesSettings {
    const KEY: &'static str = "window.processes";
}

/// The key context of the popup.
pub const POPUP_CONTEXT: &str = "ProcessesPopup";

// Strings of the reference (`IdeBundle`).
const CLICK_TO_SEE_ALL: &str = "Click to see all running processes";
const POPUP_TITLE: &str = "Processes";
const NO_PROCESSES: &str = "No processes are running.";
const ALL_COMPLETED: &str = "All background tasks completed";
const CANCEL: &str = "Cancel";
const PAUSE: &str = "Pause";
const RESUME: &str = "Resume";
const HIDE: &str = "Hide";

// The status bar widget (`InfoAndProgressPanel.MyProgressComponent`).
/// Width of the bar.
const BAR_WIDTH: f32 = 104.0;
/// Room between the text and the bar, and between the bar and the button.
const BAR_LEFT: f32 = 8.0;
const BAR_RIGHT: f32 = 4.0;
/// The gap before the buttons and after each (`BorderLayout(2, 0)`,
/// `createButtonPanel`).
const BUTTON_GAP: f32 = 2.0;
/// Size of a task's cancel button.
const BUTTON: f32 = 16.0;
/// Room before the counter (`CounterLabel`).
const COUNTER_LEFT: f32 = 4.0;

// The popup (`ProcessPopup`, `ProgressPanelBuilder`).
const POPUP_MIN_WIDTH: f32 = 300.0;
const POPUP_MAX_WIDTH: f32 = 500.0;
/// Room between the popup and the window's right edge, and above the
/// status bar.
const POPUP_RIGHT: f32 = 20.0;
const POPUP_ABOVE: f32 = 10.0;
/// The least height of the list (`POPUP_MIN_SIZE`, also when resized).
const LIST_MIN_HEIGHT: f32 = 100.0;
const LIST_TOP: f32 = 10.0;
const LIST_BOTTOM: f32 = 18.0;
const HEADER_HEIGHT: f32 = 38.0;
/// Left and right room of a row's texts.
const ROW_INSET: f32 = 13.0;
/// The separator between two rows and the room around it.
const SEPARATOR_ABOVE: f32 = 14.0;
const SEPARATOR_BELOW: f32 = 10.0;
/// The line of the bar and the button, right below the title: the
/// button's 1 + 16 is higher than the bar's 4 + bar + 4.
const BAR_ROW_HEIGHT: f32 = 17.0;
/// A row's line height for the size of its text, as a Swing label's
/// (ascent, descent and leading of the UI font).
const LINE_HEIGHT: f32 = 1.3;
const POPUP_BAR_HEIGHT: f32 = 12.0;
/// Room between a row's bar and its button.
const BUTTON_LEFT: f32 = 10.0;

/// Where a task's buttons are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    StatusBar,
    Popup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Pause,
    Cancel,
}

/// The button under the pointer, for the hovered icons and the comment
/// that names the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hover {
    spot: Spot,
    button: Button,
    id: TaskId,
}

/// The background tasks of a main window.
pub struct BackgroundTasks {
    /// The window's project.
    project: Option<PathBuf>,
    /// Every running task, in the order they started.
    tasks: Vec<TaskSnapshot>,
    popup: bool,
    /// The popup was opened while tasks ran: it closes when they end.
    close_when_done: bool,
    /// Tasks ended while the popup was open.
    finished_while_open: bool,
    hovered: Option<Hover>,
    /// The tasks shown at the last frame, for Auto Show.
    seen: Vec<TaskId>,
    /// Where the user moved and resized the popup.
    place: Place,
    drag: HeldDrag<Drag>,
    /// Where the popup was drawn last.
    laid_out: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    animation_start: Instant,
    animating: bool,
    subscription: progress::Subscription,
    _listen: Task<()>,
    show_timer: Task<()>,
    animation: Task<()>,
}

impl BackgroundTasks {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        let subscription = progress::global().subscribe(move || {
            let _ = tx.unbounded_send(());
        });
        let listen = cx.spawn_in(window, async move |this, cx| {
            while rx.next().await.is_some() {
                if this
                    .update_in(cx, |this, window, cx| this.refresh(window, cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(progress::UPDATE_INTERVAL)
                    .await;
            }
        });
        let mut this = Self {
            project: None,
            tasks: Vec::new(),
            popup: false,
            close_when_done: false,
            finished_while_open: false,
            hovered: None,
            seen: Vec::new(),
            place: Place::default(),
            drag: HeldDrag::default(),
            laid_out: Rc::default(),
            focus: cx.focus_handle(),
            return_focus: None,
            animation_start: Instant::now(),
            animating: false,
            subscription,
            _listen: listen,
            show_timer: Task::ready(()),
            animation: Task::ready(()),
        };
        this.refresh(window, cx);
        this
    }

    /// The window's project changed.
    pub fn set_project(&mut self, project: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.project != project {
            self.project = project;
            self.schedule(cx);
            cx.notify();
        }
    }

    pub fn project(&self) -> Option<&Path> {
        self.project.as_deref()
    }

    /// The tasks of the window, shown or not yet.
    fn tasks(&self) -> impl Iterator<Item = &TaskSnapshot> {
        self.tasks
            .iter()
            .filter(|t| t.belongs_to(self.project.as_deref()))
    }

    /// The tasks shown at `now`.
    pub fn shown(&self, now: Instant) -> Vec<&TaskSnapshot> {
        self.tasks().filter(|t| t.is_shown(now)).collect()
    }

    pub fn is_popup_open(&self) -> bool {
        self.popup
    }

    /// Reads the tasks again.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription.acknowledge();
        let tasks = progress::global().snapshot();
        let ended = self
            .tasks()
            .any(|t| !tasks.iter().any(|now| now.id == t.id));
        self.tasks = tasks;
        if self.popup && ended {
            self.finished_while_open = true;
            if self.close_when_done && self.tasks().next().is_none() {
                self.hide(window, cx);
            }
        }
        if let Some(hover) = self.hovered
            && !self.tasks.iter().any(|t| t.id == hover.id)
        {
            self.hovered = None;
        }
        self.schedule(cx);
        cx.notify();
    }

    /// Waits for the next task to be shown, and animates indeterminate
    /// bars while they are shown.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let wait = self
            .tasks()
            .filter(|t| !t.is_shown(now))
            .map(|t| t.until_shown(now))
            .min();
        self.show_timer = match wait {
            Some(wait) => cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(wait + Duration::from_millis(1))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.schedule(cx);
                    cx.notify();
                });
            }),
            None => Task::ready(()),
        };
        let animate = self
            .shown(now)
            .iter()
            .any(|t| t.is_indeterminate() && !t.paused);
        if animate && !self.animating {
            self.animating = true;
            self.animation = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(progress_bar::FRAME).await;
                    let go = this
                        .update(cx, |this, cx| {
                            if this.animating {
                                cx.notify();
                            }
                            this.animating
                        })
                        .unwrap_or(false);
                    if !go {
                        break;
                    }
                }
            });
        } else if !animate {
            self.animating = false;
            self.animation = Task::ready(());
        }
    }

    /// Shows the popup, or hides it when it is open.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup {
            self.hide(window, cx);
        } else {
            self.show(window, cx);
        }
    }

    /// Shows the popup and focuses it.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(true, window, cx);
    }

    /// Shows the popup, focused or not (Auto Show leaves the focus).
    fn open(&mut self, focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup {
            return;
        }
        self.popup = true;
        let running = self.tasks().next().is_some();
        self.close_when_done = running;
        self.finished_while_open = false;
        self.laid_out.set(None);
        if focus {
            self.return_focus = window.focused(cx);
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Auto Show: a second task shows in the active window
    /// (`InfoAndProgressPanel.addProgress`).
    fn auto_show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown: Vec<TaskId> = self.shown(Instant::now()).iter().map(|t| t.id).collect();
        let new = shown.iter().any(|id| !self.seen.contains(id));
        self.seen = shown;
        if new
            && self.seen.len() > 1
            && !self.popup
            && window.is_window_active()
            && settings_ui::store::get::<ProcessesSettings>(cx).auto_show
        {
            self.open(false, window, cx);
        }
    }

    /// Hides the popup; the focus goes back where it was.
    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.popup {
            return;
        }
        self.popup = false;
        self.hovered = None;
        self.drag.end();
        let return_focus = self.return_focus.take();
        if self.focus.contains_focused(window, cx)
            && let Some(focus) = return_focus
        {
            window.focus(&focus);
        }
        cx.notify();
    }

    /// Cancels a task (its cancel button).
    pub fn cancel(&mut self, id: TaskId, cx: &mut Context<Self>) -> bool {
        let cancelled = progress::global().cancel(id);
        if cancelled {
            cx.notify();
        }
        cancelled
    }

    /// Pauses or resumes a task (its pause button).
    pub fn toggle_pause(&mut self, id: TaskId, cx: &mut Context<Self>) -> bool {
        let toggled = progress::global().toggle_pause(id);
        if toggled {
            cx.notify();
        }
        toggled
    }

    fn start_move(&mut self, mouse: Point<Pixels>) {
        self.drag.start(Drag::Move {
            mouse,
            offset: self.place.offset,
        });
    }

    fn start_resize(&mut self, edges: Edges, mouse: Point<Pixels>) {
        if let Some(bounds) = self.laid_out.get() {
            self.drag.start(Drag::Resize {
                edges,
                mouse,
                bounds,
            });
        }
    }

    fn drag_to(&mut self, mouse: Point<Pixels>, viewport: Size<Pixels>, cx: &mut Context<Self>) {
        let (Some(drag), Some(laid_out)) = (self.drag.get(), self.laid_out.get()) else {
            return;
        };
        let min = size(px(POPUP_MIN_WIDTH), px(HEADER_HEIGHT + LIST_MIN_HEIGHT));
        self.place
            .drag_to(drag, mouse, laid_out, viewport, margins(cx), min);
        cx.notify();
    }

    fn set_hovered(&mut self, hover: Hover, hovered: bool, cx: &mut Context<Self>) {
        let new = if hovered {
            Some(hover)
        } else if self.hovered == Some(hover) {
            None
        } else {
            self.hovered
        };
        if new != self.hovered {
            self.hovered = new;
            cx.notify();
        }
    }

    /// The tasks and the popup for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Value {
        let now = Instant::now();
        let shown = self.shown(now);
        // What the status bar widget shows.
        let widget = shown.first().map(|first| {
            if self.popup {
                json!({ "link": hide_processes(shown.len()) })
            } else {
                json!({
                    "text": status_text(first),
                    "counter": counter_text(shown.len(), true, false),
                    "tooltip": tooltip(first),
                })
            }
        });
        json!({
            "popup": self.popup,
            "auto_show": settings_ui::store::get::<ProcessesSettings>(cx).auto_show,
            "moved": self.place.offset != Point::default(),
            "resized": self.place.size.map(|s| [f32::from(s.width), f32::from(s.height)]),
            "widget": widget,
            "tasks": self.tasks().map(|t| json!({
                "id": t.id.0,
                "title": t.title,
                "text": t.text,
                "details": t.details,
                "fraction": t.fraction,
                "cancellable": t.cancellable,
                "stopping": t.stopping,
                "pausable": t.pausable,
                "paused": t.paused,
                "shown": t.is_shown(now),
            })).collect::<Vec<_>>(),
        })
    }

    fn bar_state(&self, task: &TaskSnapshot, now: Instant) -> BarState {
        // A paused bar stands still.
        let frame = if task.paused {
            0
        } else {
            progress_bar::animation_frame(self.animation_start, now)
        };
        BarState {
            fraction: task.fraction.map(|f| f as f32),
            frame,
        }
    }

    /// The buttons of a task (`updateCancelButton`): the pause button of a
    /// pausable task, the cancel button of a cancellable one; a task with
    /// neither keeps the cancel button's room, and so does a stopping one.
    fn buttons(&self, spot: Spot, task: &TaskSnapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut buttons = Vec::new();
        if task.pausable {
            buttons.push(self.button(spot, Button::Pause, task, cx));
        }
        let painting = task.cancellable && !task.stopping;
        if painting {
            buttons.push(self.button(spot, Button::Cancel, task, cx));
        } else if !task.pausable {
            buttons.push(div().flex_none().size(px(BUTTON)).into_any_element());
        }
        buttons
    }

    /// A button of a task, its hovered icon under the pointer.
    fn button(
        &self,
        spot: Spot,
        button: Button,
        task: &TaskSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = Hover {
            spot,
            button,
            id: task.id,
        };
        let hovered = self.hovered == Some(hover);
        let (icon, tip) = match button {
            Button::Cancel if hovered => (IconName::PROGRESS_STOP_HOVERED, CANCEL),
            Button::Cancel => (IconName::PROGRESS_STOP, CANCEL),
            Button::Pause if task.paused && hovered => (IconName::PROGRESS_RESUME_HOVERED, RESUME),
            Button::Pause if task.paused => (IconName::PROGRESS_RESUME, RESUME),
            Button::Pause if hovered => (IconName::PROGRESS_PAUSE_HOVERED, PAUSE),
            Button::Pause => (IconName::PROGRESS_PAUSE, PAUSE),
        };
        let prefix = match (spot, button) {
            (Spot::StatusBar, Button::Cancel) => "status-progress-cancel",
            (Spot::StatusBar, Button::Pause) => "status-progress-pause",
            (Spot::Popup, Button::Cancel) => "processes-cancel",
            (Spot::Popup, Button::Pause) => "processes-pause",
        };
        let task_id = task.id;
        div()
            .id(SharedString::from(format!("{prefix}-{}", task.id.0)))
            .debug_selector(move || format!("{prefix}-{}", task_id.0))
            .flex_none()
            .size(px(BUTTON))
            .child(Icon::new(icon).size(px(BUTTON)))
            // In the popup the comment line names the button instead
            // (`ProgressPanelBuilder`'s icon buttons have no tooltip).
            .when(spot == Spot::StatusBar, |d| {
                d.tooltip(Tooltip::text(tip, None))
            })
            .on_hover(
                cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_hovered(hover, *hovered, cx)
                }),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                match button {
                    Button::Cancel => this.cancel(task_id, cx),
                    Button::Pause => this.toggle_pause(task_id, cx),
                };
            }))
            .into_any_element()
    }

    /// The Processes popup, while it is open.
    fn render_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let now = Instant::now();
        let theme = cx.theme();
        let status_bar = theme.metric(Metric::StatusBarHeight);
        let max_height = (window.viewport_size().height - status_bar - px(2.0 * POPUP_ABOVE))
            .max(px(HEADER_HEIGHT + LIST_MIN_HEIGHT));
        let muted = theme.color(UiRole::TextMuted);
        let disabled = theme.color(UiRole::TextDisabled);
        let separator = theme.color(UiRole::PopupBorder);
        let small = theme.ui_size() - px(2.0);
        let line_height = |size: Pixels| px((f32::from(size) * LINE_HEIGHT).round());
        let title_line = line_height(theme.ui_size());
        let small_line = line_height(small);
        let shown: Vec<TaskSnapshot> = self.shown(now).into_iter().cloned().collect();
        let mut list = div()
            .id("processes-list")
            .debug_selector(|| "processes-list".into())
            .flex()
            .flex_col()
            .min_h(px(LIST_MIN_HEIGHT))
            .pt(px(LIST_TOP))
            .pb(px(LIST_BOTTOM))
            .overflow_y_scroll();
        if shown.is_empty() {
            list = list.child(if self.finished_while_open {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_color(muted)
                    .child(Icon::new(IconName::STATUS_SUCCESS))
                    .child(ALL_COMPLETED)
            } else {
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child(NO_PROCESSES)
            });
        }
        for (i, task) in shown.iter().enumerate() {
            // The hovered button names itself in the comment line
            // (`ProgressPanel`), a paused task shows its paused text.
            let comment = match self.hovered {
                Some(h) if h.spot == Spot::Popup && h.id == task.id => match h.button {
                    Button::Cancel => CANCEL.to_owned(),
                    Button::Pause if task.paused => RESUME.to_owned(),
                    Button::Pause => PAUSE.to_owned(),
                },
                _ => text_or_paused(task).to_owned(),
            };
            let title = if task.stopping {
                stopping(&task.title)
            } else {
                task.title.clone()
            };
            let text_color = if task.stopping { disabled } else { muted };
            let buttons = self.buttons(Spot::Popup, task, cx);
            let line = |text: String| {
                div()
                    .px(px(ROW_INSET))
                    .mt(px(-1.0))
                    .h(small_line)
                    .text_size(small)
                    .line_height(small_line)
                    .text_color(text_color)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(text)
            };
            list = list.child(
                div()
                    .debug_selector(move || format!("processes-row-{i}"))
                    .flex()
                    .flex_col()
                    .when(i > 0, |d| {
                        d.child(
                            div()
                                .mt(px(SEPARATOR_ABOVE))
                                .mb(px(SEPARATOR_BELOW))
                                .h(px(1.0))
                                .bg(separator),
                        )
                    })
                    .child(
                        div()
                            .px(px(ROW_INSET))
                            .line_height(title_line)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(BAR_ROW_HEIGHT))
                            .child(
                                div()
                                    .flex_1()
                                    .ml(px(ROW_INSET))
                                    .h(px(POPUP_BAR_HEIGHT))
                                    .child(progress_bar::progress_bar(
                                        self.bar_state(task, now),
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_none()
                                    .gap(px(BUTTON_GAP))
                                    .ml(px(BUTTON_LEFT))
                                    .mr(px(ROW_INSET))
                                    .children(buttons),
                            ),
                    )
                    // Always there, empty as a space: the hint must not
                    // move the rows (`ProgressPanelBuilder.emptyComment`).
                    .child(line(comment))
                    .when(!task.details.is_empty(), |d| {
                        d.child(line(task.details.clone()))
                    }),
            );
        }
        let theme = cx.theme();
        // Moves the popup.
        let header = div()
            .id("processes-header")
            .debug_selector(|| "processes-header".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(HEADER_HEIGHT))
            .font_weight(FontWeight::SEMIBOLD)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.start_move(ev.position);
                    cx.notify();
                }),
            )
            .child(POPUP_TITLE)
            .child(
                div()
                    .absolute()
                    .right(px(10.0))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(
                        ui::components::small_icon_button(
                            "processes-hide",
                            IconName::HIDE,
                            true,
                            cx,
                        )
                        .tooltip(Tooltip::text(HIDE, None))
                        .on_mouse_down(MouseButton::Left, popup::no_move)
                        .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
                    ),
            );
        let content = div()
            .id("processes-popup")
            .debug_selector(|| "processes-popup".into())
            .key_context(POPUP_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &HideProcesses, window, cx| this.hide(window, cx)))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.color(UiRole::SurfaceRaised))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .shadow(popup_shadow())
            .text_color(theme.color(UiRole::Text))
            .text_size(theme.ui_size())
            .map(|d| match self.place.size {
                Some(_) => d.size_full(),
                // As large as the rows, within limits.
                None => d
                    .min_w(px(POPUP_MIN_WIDTH))
                    .max_w(px(POPUP_MAX_WIDTH))
                    .max_h(max_height),
            })
            .child(header)
            .child(list.flex_1());
        // At the bottom right, or where the user left it.
        let (right, bottom) = self.place.insets(margins(cx));
        let viewport = window.viewport_size();
        let owner = cx.entity();
        let cursor = match self.drag.get() {
            Some(Drag::Resize { edges, .. }) => Some(edges.cursor()),
            _ => None,
        };
        let tracking = track_held_drag(
            owner.clone(),
            |this: &mut Self| &mut this.drag,
            move |this, mouse, cx| this.drag_to(mouse, viewport, cx),
            |_, _| {},
            cursor,
        );
        let laid_out = self.laid_out.clone();
        div()
            .absolute()
            .occlude()
            .right(right)
            .bottom(bottom)
            .when_some(self.place.size, |d, s| d.w(s.width).h(s.height))
            .child(content)
            .child(
                canvas(
                    move |bounds, _, _| laid_out.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(popup::grips(owner, |this: &mut Self, edges, mouse| {
                this.start_resize(edges, mouse)
            }))
            .child(tracking)
            .into_any_element()
    }

    /// The status bar widget.
    fn render_widget(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let now = Instant::now();
        let shown = self.shown(now);
        let Some(first) = shown.first().map(|t| (*t).clone()) else {
            return div().into_any_element();
        };
        let count = shown.len();
        let theme = cx.theme();
        let hover = theme.color(UiRole::Hover);
        let widget = div()
            .id("status-progress")
            .debug_selector(|| "status-progress".into())
            .flex()
            .flex_row()
            .flex_shrink()
            .min_w(px(0.0))
            .items_center()
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .whitespace_nowrap()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)));
        if self.popup {
            return widget
                .debug_selector(|| "status-progress-hide".into())
                .child(hide_processes(count))
                .into_any_element();
        }
        let counter = counter_text(count, true, false);
        widget
            .tooltip(Tooltip::text(tooltip(&first), None))
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(status_text(&first)),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(BAR_WIDTH))
                    .h(px(22.0))
                    .ml(px(BAR_LEFT))
                    .mr(px(BAR_RIGHT))
                    .child(progress_bar::progress_bar(self.bar_state(&first, now), cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .gap(px(BUTTON_GAP))
                    .ml(px(BUTTON_GAP))
                    .mr(px(BUTTON_GAP))
                    .children(self.buttons(Spot::StatusBar, &first, cx)),
            )
            .when(!counter.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "status-progress-counter".into())
                        .flex_none()
                        .pl(px(COUNTER_LEFT))
                        .child(counter),
                )
            })
            .into_any_element()
    }
}

impl Render for BackgroundTasks {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.auto_show(window, cx);
        self.render_widget(cx)
    }
}

impl Focusable for BackgroundTasks {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The Processes popup of a window's [`BackgroundTasks`], drawn over the
/// window.
pub struct ProcessesPopup {
    tasks: Entity<BackgroundTasks>,
    _observe: gpui::Subscription,
}

impl ProcessesPopup {
    pub fn new(tasks: Entity<BackgroundTasks>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&tasks, |_, _, cx| cx.notify());
        Self {
            tasks,
            _observe: observe,
        }
    }
}

impl Render for ProcessesPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.tasks.read(cx).popup {
            return div().into_any_element();
        }
        self.tasks.update(cx, |t, cx| t.render_popup(window, cx))
    }
}

/// Handles the commands on the window's root element.
pub(crate) fn on_actions<E: InteractiveElement>(root: E, tasks: &Entity<BackgroundTasks>) -> E {
    let show = tasks.clone();
    let hide = tasks.clone();
    let cancel = tasks.clone();
    let start = tasks.clone();
    let pause = tasks.clone();
    root.on_action(move |_: &ShowProcesses, window, cx| {
        show.update(cx, |t, cx| t.toggle(window, cx))
    })
    .on_action(move |_: &HideProcesses, window, cx| hide.update(cx, |t, cx| t.hide(window, cx)))
    .on_action(move |a: &CancelBackgroundTask, _, cx| {
        cancel.update(cx, |t, cx| {
            t.cancel(TaskId(a.id), cx);
        })
    })
    .on_action(move |a: &PauseBackgroundTask, _, cx| {
        pause.update(cx, |t, cx| {
            t.toggle_pause(TaskId(a.id), cx);
        })
    })
    .on_action(|_: &ToggleAutoShowProcesses, _, cx| {
        let mut settings = settings_ui::store::get::<ProcessesSettings>(cx);
        settings.auto_show = !settings.auto_show;
        settings_ui::store::set(cx, &settings);
    })
    .on_action(move |a: &StartBackgroundTask, _, cx| {
        let project = start.read(cx).project.clone();
        start_sample(a, project);
    })
}

/// The room the popup's default place leaves to the window's right edge
/// and its bottom edge (`ProcessPopup.calculateBounds`: 20 from the right,
/// 10 above the status bar).
fn margins(cx: &App) -> Margins {
    Margins {
        right: px(POPUP_RIGHT),
        bottom: cx.theme().metric(Metric::StatusBarHeight) + px(POPUP_ABOVE),
    }
}

/// The text of a task, its paused text while paused.
fn text_or_paused(task: &TaskSnapshot) -> &str {
    if task.paused && !task.paused_text.is_empty() {
        &task.paused_text
    } else {
        &task.text
    }
}

/// Runs the task of [`StartBackgroundTask`] on a thread of its own.
fn start_sample(a: &StartBackgroundTask, project: Option<PathBuf>) {
    let mut builder = progress::global().task(a.title.clone());
    if let Some(project) = project {
        builder = builder.project(project);
    }
    if a.cancellable.unwrap_or(true) {
        builder = builder.cancellable();
    }
    if let Some(text) = &a.text {
        builder = builder.text(text);
    }
    if let Some(paused) = &a.paused_text {
        builder = builder.pausable(paused);
    }
    let task = builder.start();
    if let Some(details) = &a.details {
        task.set_details(details);
    }
    if let Some(fraction) = a.fraction {
        task.set_fraction(fraction);
    }
    let seconds = a.seconds.unwrap_or(10.0).clamp(0.0, 3600.0);
    let end = Instant::now() + Duration::from_secs_f64(seconds);
    std::thread::spawn(move || {
        while Instant::now() < end && !task.is_cancelled() {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(task);
    });
}

/// The text of the status bar widget: the task's text, its title without
/// one, "Stopping - …" once cancelled (`ProgressComponent.updateProgressNow`).
pub fn status_text(task: &TaskSnapshot) -> String {
    let text = text_or_paused(task);
    let text = if text.is_empty() { &task.title } else { text };
    if task.stopping {
        stopping(text)
    } else {
        text.to_owned()
    }
}

fn stopping(text: &str) -> String {
    format!("Stopping - {text}")
}

/// The tooltip of the widget (`ProgressComponent.computeTooltipText`).
pub fn tooltip(task: &TaskSnapshot) -> String {
    format!("{}. {CLICK_TO_SEE_ALL}", task.title)
}

/// The link of the widget while the popup is open.
pub fn hide_processes(count: usize) -> String {
    format!("Hide processes ({count})")
}

/// The counter after the widget's task (`CounterLabel`): `+N` for the
/// other tasks while a task is shown, nothing while the popup is open.
pub fn counter_text(count: usize, indicator_visible: bool, popup: bool) -> String {
    let others = if indicator_visible {
        count.saturating_sub(1)
    } else {
        count
    };
    if popup || others == 0 || (!indicator_visible && others == 1) {
        String::new()
    } else if indicator_visible {
        format!("+{others}")
    } else {
        others.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, text: &str) -> TaskSnapshot {
        let tasks = progress::Tasks::new();
        let t = tasks.task(title).text(text).cancellable().start();
        let snapshot = tasks.snapshot().remove(0);
        drop(t);
        snapshot
    }

    #[test]
    fn counter_counts_the_other_tasks() {
        assert_eq!(counter_text(1, true, false), "");
        assert_eq!(counter_text(2, true, false), "+1");
        assert_eq!(counter_text(12, true, false), "+11");
        assert_eq!(counter_text(3, true, true), "");
        assert_eq!(counter_text(1, false, false), "");
        assert_eq!(counter_text(2, false, false), "2");
    }

    #[test]
    fn widget_text_falls_back_to_the_title() {
        let mut t = task("Scanning files…", "");
        assert_eq!(status_text(&t), "Scanning files…");
        t.text = "Indexing app".into();
        assert_eq!(status_text(&t), "Indexing app");
        t.stopping = true;
        assert_eq!(status_text(&t), "Stopping - Indexing app");
    }

    #[test]
    fn tooltip_and_link_texts() {
        let t = task("Updating Python interpreter", "");
        assert_eq!(
            tooltip(&t),
            "Updating Python interpreter. Click to see all running processes"
        );
        assert_eq!(hide_processes(2), "Hide processes (2)");
    }
}
