//! The language servers' work as background tasks (`progress`), shown in
//! the status bar: a server starting, until it answers `initialize`, and
//! the work it reports with `$/progress` (its title, message and
//! percentage), until the work ends or the server stops.

use std::collections::HashMap;
use std::path::Path;

use code_intel::{IntelEvent, ServerId, ServerState};

/// The tasks of a project's servers.
#[derive(Default)]
pub(crate) struct ServerTasks {
    starting: HashMap<ServerId, progress::Task>,
    work: HashMap<(ServerId, String), progress::Task>,
}

/// The title of the task of a server starting.
pub fn starting_title(name: &str) -> String {
    format!("Starting {name}\u{2026}")
}

impl ServerTasks {
    /// Follows an event of the services of the project at `root`.
    pub fn event(&mut self, event: &IntelEvent, root: Option<&Path>) {
        self.event_in(progress::global(), event, root);
    }

    fn event_in(&mut self, tasks: &progress::Tasks, event: &IntelEvent, root: Option<&Path>) {
        let start = |title: String| {
            let task = tasks.task(title);
            match root {
                Some(root) => task.project(root),
                None => task,
            }
            .start()
        };
        match event {
            IntelEvent::Status(status) => {
                if status.state == ServerState::Initializing {
                    self.starting
                        .entry(status.id.clone())
                        .or_insert_with(|| start(starting_title(&status.name)));
                } else {
                    self.starting.remove(&status.id);
                }
                if !status.state.is_running() {
                    self.work.retain(|(id, _), _| *id != status.id);
                }
            }
            IntelEvent::Progress {
                server,
                title,
                message,
                percentage,
                done,
            } => {
                let key = (server.clone(), title.clone());
                if *done {
                    self.work.remove(&key);
                    return;
                }
                let task = self.work.entry(key).or_insert_with(|| start(title.clone()));
                task.set_text(message.as_deref().unwrap_or_default());
                match percentage {
                    Some(p) => task.set_fraction(f64::from(*p) / 100.0),
                    None => task.set_indeterminate(),
                }
            }
            _ => {}
        }
    }

    /// The project's services are gone.
    pub fn clear(&mut self) {
        self.starting.clear();
        self.work.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use code_intel::ServerStatus;

    fn status(state: ServerState) -> IntelEvent {
        let mut status = ServerStatus::new(ServerId::new("ty"), "ty");
        status.state = state;
        IntelEvent::Status(status)
    }

    fn progress(message: Option<&str>, percentage: Option<u32>, done: bool) -> IntelEvent {
        IntelEvent::Progress {
            server: ServerId::new("ty"),
            title: "Indexing".into(),
            message: message.map(Into::into),
            percentage,
            done,
        }
    }

    #[test]
    fn a_starting_server_is_a_task_until_it_runs() {
        let tasks = progress::Tasks::new();
        let mut server = ServerTasks::default();
        let root = Path::new("/p");
        server.event_in(&tasks, &status(ServerState::Initializing), Some(root));
        server.event_in(&tasks, &status(ServerState::Initializing), Some(root));
        let shown = tasks.snapshot();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Starting ty\u{2026}");
        assert_eq!(shown[0].project.as_deref(), Some(root));
        server.event_in(&tasks, &status(ServerState::Running), Some(root));
        assert!(tasks.is_empty());
    }

    #[test]
    fn reported_work_follows_the_server() {
        let tasks = progress::Tasks::new();
        let mut server = ServerTasks::default();
        server.event_in(&tasks, &progress(Some("3/10 files"), Some(30), false), None);
        let shown = tasks.snapshot();
        assert_eq!(shown[0].title, "Indexing");
        assert_eq!(shown[0].text, "3/10 files");
        assert_eq!(shown[0].fraction, Some(0.3));
        server.event_in(&tasks, &progress(None, None, false), None);
        assert_eq!(tasks.snapshot()[0].fraction, None);
        server.event_in(&tasks, &progress(None, None, true), None);
        assert!(tasks.is_empty());
        // A server that stops ends its work.
        server.event_in(&tasks, &progress(None, Some(10), false), None);
        server.event_in(&tasks, &status(ServerState::Stopped), None);
        assert!(tasks.is_empty());
    }
}
