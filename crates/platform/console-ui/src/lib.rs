//! The console view of Pyoxide IDE on GPUI (ADR 0007, 0015), shared by the
//! Run tool window and the Console tab of a debug session:
//!
//! - [`ConsoleView`]: the `console` crate's text model shown in a read-only
//!   editor, typed input sent to the process's standard input;
//! - [`feed`]: a process's output into a console's model off the UI thread,
//!   then the exit line, as the reference's `ConsoleViewImpl` prints it
//!   when the process handler ends;
//! - [`feed_input`]: a file as a process's standard input ("Redirect input
//!   from").

mod decor;
mod view;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use console::ContentKind;
use futures::channel::mpsc;
use gpui::{App, Entity};
use process::{ExitStatus, ProcessEvent, ProcessHandle};

pub use view::{ConsoleEvent, ConsoleView, FLOOD_INTERVAL, FLUSH_DELAY, ansi_color, role_color};

/// What [`feed`] tells the console's owner: new output waits for the view
/// (`None`: call [`ConsoleView::request_flush`]), or the process ended
/// with the exit line printed.
pub type FeedMessage = Option<ExitStatus>;

/// Feeds a process's output into a console's model off the UI thread (link
/// filters may look at the file system) and prints "Process finished with
/// exit code …" when it ends. The owner shows the output when told to and
/// learns the exit status from the returned channel.
pub fn feed(
    console: &Entity<ConsoleView>,
    events: flume::Receiver<ProcessEvent>,
    cx: &App,
) -> mpsc::UnboundedReceiver<FeedMessage> {
    let (model, dirty) = {
        let c = console.read(cx);
        (c.model(), c.dirty_flag())
    };
    let (tx, rx) = mpsc::unbounded::<FeedMessage>();
    cx.background_executor()
        .spawn(async move {
            while let Ok(event) = events.recv_async().await {
                let mut model = model.lock().unwrap_or_else(|e| e.into_inner());
                match event {
                    ProcessEvent::Stdout(bytes) => model.push(ContentKind::Normal, &bytes),
                    ProcessEvent::Stderr(bytes) => model.push(ContentKind::Error, &bytes),
                    ProcessEvent::Exited(status) => {
                        model.finish();
                        model.push_str(
                            ContentKind::System,
                            &format!("\n{}\n", status.finished_message()),
                        );
                        drop(model);
                        let _ = tx.unbounded_send(Some(status));
                        return;
                    }
                }
                drop(model);
                if !dirty.swap(true, Ordering::AcqRel) {
                    let _ = tx.unbounded_send(None);
                }
            }
        })
        .detach();
    rx
}

/// Writes a file to a process's standard input and closes it, off the UI
/// thread.
pub fn feed_input(process: ProcessHandle, file: PathBuf, cx: &App) {
    cx.background_executor()
        .spawn(async move {
            if let Ok(bytes) = std::fs::read(&file) {
                let _ = process.write_stdin(&bytes);
            }
            process.close_stdin();
        })
        .detach();
}
