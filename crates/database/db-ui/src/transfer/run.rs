//! Running an export or an import: the job on the database runtime, a
//! cancellable background task for the status bar, and the balloon that
//! tells how it ended. Nothing here waits: the jobs report through a
//! channel that a task of the application drains.

use std::path::Path;
use std::sync::Arc;

use db_session::{
    ExportEvent, ExportOutcome, ExportRequest, ExportSink, ImportEvent, ImportOutcome,
    ImportRequest, ImportSink,
};
use futures::StreamExt as _;
use gpui::{AnyWindowHandle, App, ClipboardItem};
use serde_json::json;
use ui::notification::{Notification, NotificationKind};
use workbench::Workbench;

use super::TransferTarget;
use crate::grid;

/// Where an export goes.
#[derive(Debug, Clone)]
pub(crate) enum Goal {
    File,
    Clipboard,
}

/// The text of the balloon's link to a file.
pub(crate) fn reveal_text() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in Files"
    }
}

/// Shows `notification` in the main window `window`.
pub(crate) fn notify(window: AnyWindowHandle, notification: Notification, cx: &mut App) {
    if let Some(handle) = window.downcast::<Workbench>() {
        let _ = handle.update(cx, |wb, _, cx| {
            wb.notify(notification, cx);
        });
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn rows_text(rows: u64) -> String {
    format!("{rows} {}", if rows == 1 { "row" } else { "rows" })
}

fn millis(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_millis();
    if ms >= 10_000 {
        format!("{} s", ms / 1000)
    } else {
        format!("{ms} ms")
    }
}

/// Starts an export of `request` for `target`; the end is a balloon in
/// `window` (the main window), and for [`Goal::Clipboard`] the text goes to
/// the clipboard first. The work shows in the status bar's background tasks
/// with a cancel button.
pub(crate) fn export(
    target: &TransferTarget,
    request: ExportRequest,
    goal: Goal,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<ExportEvent>();
    let sink: ExportSink = Arc::new(move |e| {
        let _ = tx.unbounded_send(e);
    });
    let handle = target.target.handle.clone();
    let job = handle.export(request, sink);
    let name = target.target.table.name.clone();
    let task = progress::global()
        .task(format!("Exporting {name}"))
        .cancellable()
        .start();
    {
        let handle = handle.clone();
        task.cancel_token()
            .on_cancel(move || handle.cancel_job(job));
    }
    let source = target.target.source.clone();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            match event {
                ExportEvent::Progress { rows, .. } => task.set_text(rows_text(rows)),
                ExportEvent::Finished { outcome, .. } => {
                    task.finish();
                    let _ = cx.update(|cx| match outcome {
                        ExportOutcome::Done(done) => {
                            let rows = rows_text(done.rows);
                            let took = millis(done.elapsed);
                            match (goal, done.path, done.text) {
                                (Goal::File, Some(path), _) => {
                                    let notification = Notification::new(
                                        NotificationKind::Information,
                                        source,
                                        format!(
                                            "{rows} saved from {name} to {} ({took})",
                                            file_name(&path)
                                        ),
                                    )
                                    .with_action(
                                        reveal_text(),
                                        "project.reveal-file",
                                        json!({ "path": path.display().to_string() }),
                                    );
                                    notify(window, notification, cx);
                                }
                                (Goal::Clipboard, _, Some(text)) => {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    notify(
                                        window,
                                        Notification::new(
                                            NotificationKind::Information,
                                            source,
                                            format!(
                                                "{rows} of {name} copied to the clipboard ({took})"
                                            ),
                                        ),
                                        cx,
                                    );
                                }
                                _ => {}
                            }
                        }
                        ExportOutcome::Cancelled => {}
                        ExportOutcome::Failed(e) => notify(
                            window,
                            Notification::new(
                                NotificationKind::Error,
                                source,
                                format!("Cannot export {name}: {}", e.message),
                            ),
                            cx,
                        ),
                    });
                    break;
                }
            }
        }
    })
    .detach();
}

/// Starts the import of `request` into `target`'s table; the end is a
/// balloon in `window`, and grids of the table showing old rows are read
/// again.
pub(crate) fn import(
    target: &TransferTarget,
    request: ImportRequest,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<ImportEvent>();
    let sink: ImportSink = Arc::new(move |e| {
        let _ = tx.unbounded_send(e);
    });
    let handle = target.target.handle.clone();
    let file = file_name(&request.file);
    let job = handle.import(request, sink);
    let name = target.target.table.name.clone();
    let task = progress::global()
        .task(format!("Importing {file}"))
        .cancellable()
        .start();
    {
        let handle = handle.clone();
        task.cancel_token()
            .on_cancel(move || handle.cancel_job(job));
    }
    let source = target.target.source.clone();
    let table = target.target.table.clone();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            match event {
                ImportEvent::Progress {
                    rows, bytes, total, ..
                } => {
                    task.set_text(rows_text(rows));
                    if total > 0 {
                        task.set_fraction(bytes as f64 / total as f64);
                    }
                }
                ImportEvent::Finished { outcome, .. } => {
                    task.finish();
                    let _ = cx.update(|cx| match outcome {
                        ImportOutcome::Done(done) => {
                            notify(
                                window,
                                Notification::new(
                                    NotificationKind::Information,
                                    source.clone(),
                                    format!(
                                        "{file} imported to {name}: {} ({})",
                                        rows_text(done.rows),
                                        millis(done.elapsed)
                                    ),
                                ),
                                cx,
                            );
                            grid::reload_table(window, &source, &table, cx);
                        }
                        ImportOutcome::Cancelled => {}
                        ImportOutcome::Failed(f) => notify(
                            window,
                            Notification::new(
                                NotificationKind::Error,
                                source,
                                format!(
                                    "Cannot import {file} into {name}: {f}. The table is unchanged."
                                ),
                            ),
                            cx,
                        ),
                    });
                    break;
                }
            }
        }
    })
    .detach();
}
