//! Closing the main window, Close Project and Quit (#401): what the
//! window started ends with it ([`ui::window_close`]: run and debug
//! sessions, terminal sessions, background tasks, the window's dialogs and
//! popups). A process that runs is asked about first, one at a time, as
//! the reference's run contents do when their project closes
//! (`BaseContentCloseListener.canClose`, `TerminateRemoteProcessDialog`):
//! "Process 'x' Is Running" / "Do you want to terminate the process 'x'?",
//! Terminate, Disconnect or Cancel, and "Don't ask again" (the choice is
//! then made for every process closed from now on, tabs too). Terminate
//! ends the process with its children, Disconnect leaves it running
//! without the IDE (a debug session detaches); the window closes once
//! they are done (at most [`WAIT`]). Cancel keeps the window open and
//! stops quitting.

use std::collections::VecDeque;
use std::time::Duration;

use futures::FutureExt as _;
use gpui::{AnyWindowHandle, App, AppContext as _, Context, Entity, Task, Window, WindowHandle};
use ui::icons::IconName;
use ui::window_close::{Running, Stop};

use super::Workbench;
use super::run_actions::{CloseRunning, PendingDialog};
use crate::dialog::{Dialog, DialogButton};
use crate::run::RunToolWindow;

/// How long closing waits for the processes and parts to end before the
/// window closes anyway (what still runs is killed by then).
const WAIT: Duration = Duration::from_secs(5);

/// What closes.
pub(crate) enum Closing {
    /// The window's close button.
    Window,
    /// Close Project.
    Project,
    /// The application quits: `then` goes on with the next window.
    Quit(Box<dyn FnOnce(&mut App)>),
}

/// A close in progress: the processes still to ask about and the stops
/// already chosen.
pub(super) struct InProgress {
    closing: Closing,
    queue: VecDeque<Running>,
    stops: Vec<Task<()>>,
}

impl Workbench {
    /// Closes the window (or its project, or goes on quitting) once every
    /// running process was answered for.
    pub(crate) fn close_window(
        &mut self,
        closing: Closing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing.is_some() {
            return;
        }
        let queue = ui::window_close::running(window.window_handle(), cx).into();
        self.closing = Some(InProgress {
            closing,
            queue,
            stops: Vec::new(),
        });
        self.ask_next_running(window, cx);
    }

    fn ask_next_running(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        loop {
            let Some(next) = self.closing.as_mut().and_then(|c| c.queue.pop_front()) else {
                break;
            };
            let how = match self.run_settings.close_running {
                CloseRunning::Terminate => Stop::Terminate,
                CloseRunning::Disconnect => Stop::Disconnect,
                CloseRunning::Ask => {
                    self.ask_running(next, window, cx);
                    return;
                }
            };
            let stop = next.stop(how, cx);
            if let Some(closing) = self.closing.as_mut() {
                closing.stops.push(stop);
            }
        }
        if let Some(closing) = self.closing.take() {
            self.finish_close(closing, window, cx);
        }
    }

    fn ask_running(&mut self, running: Running, window: &mut Window, cx: &mut Context<Self>) {
        let name = running.name().to_owned();
        let dialog = cx.new(|cx| {
            Dialog::new(
                format!("Process '{name}' Is Running"),
                format!("Do you want to terminate the process '{name}'?"),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "disconnect",
                        label: "&Disconnect".into(),
                    },
                    DialogButton {
                        id: "terminate",
                        label: "&Terminate".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
            .with_option("Don't ask again")
        });
        self.open_dialog(dialog, PendingDialog::CloseWindow(running), window, cx);
    }

    /// The answer about one running process.
    pub(super) fn close_window_answered(
        &mut self,
        running: Running,
        button: Option<&'static str>,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let how = match button {
            Some("terminate") => Stop::Terminate,
            Some("disconnect") => Stop::Disconnect,
            // Cancel: the window stays (what was stopped already is gone).
            _ => {
                self.closing = None;
                return;
            }
        };
        if checked {
            self.run_settings.close_running = match how {
                Stop::Terminate => CloseRunning::Terminate,
                Stop::Disconnect => CloseRunning::Disconnect,
            };
        }
        let stop = running.stop(how, cx);
        if let Some(closing) = self.closing.as_mut() {
            closing.stops.push(stop);
        }
        self.ask_next_running(window, cx);
    }

    /// Every process was answered for: the stops end, then the window's
    /// parts and dialogs, then the window closes.
    fn finish_close(&mut self, closing: InProgress, window: &mut Window, cx: &mut Context<Self>) {
        let InProgress { closing, stops, .. } = closing;
        if !matches!(closing, Closing::Quit(_)) {
            self.save_all(cx);
        }
        let handle = window.window_handle();
        cx.spawn_in(window, async move |this, cx| {
            wait_all(stops, cx).await;
            match closing {
                // The application's quit ends the rest.
                Closing::Quit(then) => {
                    let _ = cx.update(|_, cx| then(cx));
                }
                closing => {
                    let Ok(ends) = cx.update(|_, cx| ui::window_close::end(handle, cx)) else {
                        return;
                    };
                    wait_all(ends, cx).await;
                    let _ = this.update_in(cx, |this, window, cx| match closing {
                        Closing::Project => this.close_project_now(window, cx),
                        _ => window.remove_window(),
                    });
                }
            }
        })
        .detach();
    }

    /// Registers what the window starts with [`ui::window_close`]: the
    /// runs and debug sessions (asked about), the terminal sessions with
    /// what they started, the project's background tasks (cancelled).
    pub(crate) fn register_window_parts(this: &Entity<Self>, window: &Window, cx: &mut App) {
        let handle = window.window_handle();
        let (run, debugger, terminal) = {
            let w = this.read(cx);
            (
                w.run_window.downgrade(),
                w.debugger.downgrade(),
                w.terminal_ui.view.downgrade(),
            )
        };
        let workbench = this.downgrade();
        let runs = ui::window_close::register_fn(
            handle,
            {
                let run = run.clone();
                move |cx| {
                    run.upgrade()
                        .map(|r| RunToolWindow::running_processes(&r, cx))
                        .unwrap_or_default()
                }
            },
            move |cx| {
                let _ = run.update(cx, |r, _| r.kill_all());
                Task::ready(())
            },
            cx,
        );
        let debug = ui::window_close::register_fn(
            handle,
            {
                let debugger = debugger.clone();
                move |cx| {
                    debugger
                        .upgrade()
                        .map(|d| d.read(cx).running())
                        .unwrap_or_default()
                }
            },
            move |cx| {
                if let Some(d) = debugger.upgrade() {
                    d.read(cx).kill_all();
                }
                Task::ready(())
            },
            cx,
        );
        let terminals = ui::window_close::register_fn(
            handle,
            |_| Vec::new(),
            move |cx| {
                let Some(view) = terminal.upgrade() else {
                    return Task::ready(());
                };
                let pids: Vec<u32> = view
                    .read(cx)
                    .tabs()
                    .iter()
                    .filter_map(|t| t.session().read(cx).session().map(|s| s.pid()))
                    .collect();
                if pids.is_empty() {
                    return Task::ready(());
                }
                cx.background_spawn(async move {
                    terminal::end_sessions(&pids, terminal::QUIT_GRACE);
                })
            },
            cx,
        );
        let tasks = ui::window_close::register_fn(
            handle,
            |_| Vec::new(),
            move |cx| {
                let root = workbench
                    .upgrade()
                    .and_then(|w| w.read(cx).project_root(cx));
                if let Some(root) = root {
                    let tasks = progress::global();
                    for task in tasks.snapshot() {
                        if task.project.as_deref() == Some(root.as_path()) {
                            tasks.cancel(task.id);
                        }
                    }
                }
                Task::ready(())
            },
            cx,
        );
        this.update(cx, |w, _| {
            w.window_parts = vec![runs, debug, terminals, tasks]
        });
    }
}

/// Waits for `tasks`, at most [`WAIT`].
async fn wait_all(tasks: Vec<Task<()>>, cx: &mut gpui::AsyncWindowContext) {
    if tasks.is_empty() {
        return;
    }
    let timeout = cx.background_executor().timer(WAIT);
    futures::select! {
        _ = futures::future::join_all(tasks).fuse() => {}
        _ = timeout.fuse() => {}
    }
}

/// The window's close button: changes in its Settings window are settled
/// first, then the running processes are asked about. True when the
/// window may close now (nothing to ask: its parts end as it closes).
pub(crate) fn should_close(window: &mut Window, cx: &mut App) -> bool {
    let handle = window.window_handle();
    let settled = settings_ui::leave::settle_parent(handle, cx, move |cx| {
        request_close(handle, Closing::Window, cx)
    });
    if !settled {
        return false;
    }
    if !ui::window_close::running(handle, cx).is_empty() {
        cx.defer(move |cx| request_close(handle, Closing::Window, cx));
        return false;
    }
    if let Some(workbench) = window.root::<Workbench>().flatten() {
        workbench.update(cx, |w, cx| w.save_all(cx));
    }
    for task in ui::window_close::end(handle, cx) {
        task.detach();
    }
    true
}

/// Starts closing the main window `handle`.
pub(crate) fn request_close(handle: AnyWindowHandle, closing: Closing, cx: &mut App) {
    let Some(workbench) = handle.downcast::<Workbench>() else {
        return;
    };
    let _ = workbench.update(cx, |w, window, cx| w.close_window(closing, window, cx));
}

/// Quit: every main window asks about its running processes in turn, then
/// the application quits; Cancel in one of them stops quitting.
pub(crate) fn quit(cx: &mut App) {
    let windows: Vec<WindowHandle<Workbench>> = cx
        .windows()
        .iter()
        .filter_map(|w| w.downcast::<Workbench>())
        .collect();
    quit_next(windows, cx);
}

fn quit_next(mut windows: Vec<WindowHandle<Workbench>>, cx: &mut App) {
    let Some(window) = windows.pop() else {
        cx.quit();
        return;
    };
    let closing = Closing::Quit(Box::new(move |cx| quit_next(windows, cx)));
    // A window gone meanwhile is skipped.
    let mut closing = Some(closing);
    let _ = window.update(cx, |w, window, cx| {
        if let Some(closing) = closing.take() {
            w.close_window(closing, window, cx);
        }
    });
    if let Some(Closing::Quit(then)) = closing {
        then(cx);
    }
}
