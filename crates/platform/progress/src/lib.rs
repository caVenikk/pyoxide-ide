//! Background tasks without a UI toolkit: what the IDE does in the
//! background (scanning the project, starting a language server, a git
//! fetch), as the status bar and the Background Tasks popup show it.
//!
//! A [`Task`] is started from a [`Tasks`] registry with a title and,
//! optionally, a project and a cancel button. The work updates its text,
//! details and fraction from any thread; the first fraction makes it
//! determinate. Cancelling sets the task's [`CancelToken`], which the work
//! checks (or forwards to its own cancellation with
//! [`CancelToken::on_cancel`]); the task stays, "stopping", until the work
//! ends. A task ends when its handle is dropped.
//!
//! Observers (the status bar of each main window) [`Tasks::subscribe`]
//! with a waker: it is called once after a change, not again until the
//! observer [acknowledges](Subscription::acknowledge) it, so updates from
//! busy threads coalesce and the observer reads a [`Tasks::snapshot`] at
//! its own pace (the reference repaints at most every
//! [`UPDATE_INTERVAL`]). A task is shown only once it has run for
//! [`SHOW_DELAY`], so that quick work does not flicker.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

/// How long a task runs before it is shown
/// (`ProgressUIUtil.DEFAULT_PROGRESS_DELAY_MILLIS`).
pub const SHOW_DELAY: Duration = Duration::from_millis(300);

/// The shortest time between two updates of what observers show (the
/// reference throttles indicator updates to 50 ms).
pub const UPDATE_INTERVAL: Duration = Duration::from_millis(50);

/// The registry of the application's tasks, shared by every main window.
pub fn global() -> &'static Tasks {
    static GLOBAL: OnceLock<Tasks> = OnceLock::new();
    GLOBAL.get_or_init(Tasks::new)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The identity of a task, unique in its registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u64);

/// A registry of running tasks. Cloning shares the registry.
#[derive(Clone, Default)]
pub struct Tasks {
    inner: Arc<Registry>,
}

#[derive(Default)]
struct Registry {
    tasks: Mutex<Vec<Arc<Shared>>>,
    subscribers: Mutex<Vec<Arc<Subscriber>>>,
    next_id: AtomicU64,
}

struct Subscriber {
    pending: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

struct Shared {
    id: TaskId,
    title: String,
    project: Option<PathBuf>,
    cancellable: bool,
    started: Instant,
    state: Mutex<State>,
    cancel: CancelToken,
}

#[derive(Debug, Clone, Default)]
struct State {
    text: String,
    details: String,
    fraction: Option<f64>,
}

impl Registry {
    /// Wakes every observer that has not been woken since it last looked.
    fn changed(&self) {
        let subscribers = lock(&self.subscribers).clone();
        for s in subscribers {
            if !s.pending.swap(true, Ordering::AcqRel) {
                (s.wake)();
            }
        }
    }

    fn remove(&self, id: TaskId) {
        let removed = {
            let mut tasks = lock(&self.tasks);
            let before = tasks.len();
            tasks.retain(|t| t.id != id);
            tasks.len() != before
        };
        if removed {
            self.changed();
        }
    }
}

impl Tasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Describes a new task; [`TaskBuilder::start`] starts it.
    pub fn task(&self, title: impl Into<String>) -> TaskBuilder<'_> {
        TaskBuilder {
            tasks: self,
            title: title.into(),
            project: None,
            cancellable: false,
            text: String::new(),
            cancel: None,
        }
    }

    /// The running tasks in the order they started.
    pub fn snapshot(&self) -> Vec<TaskSnapshot> {
        lock(&self.inner.tasks)
            .iter()
            .map(|t| t.snapshot())
            .collect()
    }

    /// Number of running tasks.
    pub fn len(&self) -> usize {
        lock(&self.inner.tasks).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Cancels a task (its cancel button): its token is set and the task
    /// shows as stopping until its work ends. `false` when there is no
    /// such task or it cannot be cancelled.
    pub fn cancel(&self, id: TaskId) -> bool {
        let task = lock(&self.inner.tasks)
            .iter()
            .find(|t| t.id == id)
            .cloned();
        match task {
            Some(t) if t.cancellable && !t.cancel.is_cancelled() => {
                t.cancel.cancel();
                self.inner.changed();
                true
            }
            _ => false,
        }
    }

    /// Calls `wake` (from whatever thread changed the tasks) after the
    /// tasks change; not again until [`Subscription::acknowledge`]. The
    /// observer stops when the subscription is dropped.
    pub fn subscribe(&self, wake: impl Fn() + Send + Sync + 'static) -> Subscription {
        let subscriber = Arc::new(Subscriber {
            pending: AtomicBool::new(false),
            wake: Box::new(wake),
        });
        lock(&self.inner.subscribers).push(subscriber.clone());
        Subscription {
            subscriber,
            registry: Arc::downgrade(&self.inner),
        }
    }
}

impl fmt::Debug for Tasks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tasks")
            .field("tasks", &self.snapshot())
            .finish()
    }
}

/// A task about to start.
#[must_use = "a task starts with `start`"]
pub struct TaskBuilder<'a> {
    tasks: &'a Tasks,
    title: String,
    project: Option<PathBuf>,
    cancellable: bool,
    text: String,
    cancel: Option<CancelToken>,
}

impl TaskBuilder<'_> {
    /// The task belongs to the project at `root`: only that project's
    /// windows show it. Without a project every main window shows it.
    pub fn project(mut self, root: impl Into<PathBuf>) -> Self {
        self.project = Some(root.into());
        self
    }

    /// The task has a cancel button.
    pub fn cancellable(mut self) -> Self {
        self.cancellable = true;
        self
    }

    /// The first text.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Cancelling the task sets `token` (the work's own token).
    pub fn cancel_token(mut self, token: CancelToken) -> Self {
        self.cancel = Some(token);
        self
    }

    pub fn start(self) -> Task {
        let registry = &self.tasks.inner;
        let id = TaskId(registry.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let shared = Arc::new(Shared {
            id,
            title: self.title,
            project: self.project,
            cancellable: self.cancellable,
            started: Instant::now(),
            state: Mutex::new(State {
                text: self.text,
                ..State::default()
            }),
            cancel: self.cancel.unwrap_or_default(),
        });
        lock(&registry.tasks).push(shared.clone());
        registry.changed();
        Task {
            shared,
            registry: Arc::downgrade(registry),
        }
    }
}

/// A running task. It ends when dropped. The setters may be called from
/// any thread; an unchanged value does not wake the observers.
pub struct Task {
    shared: Arc<Shared>,
    registry: Weak<Registry>,
}

impl Task {
    pub fn id(&self) -> TaskId {
        self.shared.id
    }

    pub fn title(&self) -> &str {
        &self.shared.title
    }

    fn update(&self, f: impl FnOnce(&mut State) -> bool) {
        let changed = f(&mut lock(&self.shared.state));
        if changed && let Some(registry) = self.registry.upgrade() {
            registry.changed();
        }
    }

    /// What the task does now (`Scanning files…`).
    pub fn set_text(&self, text: impl AsRef<str>) {
        let text = text.as_ref();
        self.update(|s| {
            if s.text == text {
                return false;
            }
            text.clone_into(&mut s.text);
            true
        });
    }

    /// The second line: the item at hand (a folder, a file).
    pub fn set_details(&self, details: impl AsRef<str>) {
        let details = details.as_ref();
        self.update(|s| {
            if s.details == details {
                return false;
            }
            details.clone_into(&mut s.details);
            true
        });
    }

    /// How much is done, from 0 to 1; the task becomes determinate.
    pub fn set_fraction(&self, fraction: f64) {
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        self.update(|s| {
            if s.fraction == Some(fraction) {
                return false;
            }
            s.fraction = Some(fraction);
            true
        });
    }

    /// The amount done is not known (again).
    pub fn set_indeterminate(&self) {
        self.update(|s| s.fraction.take().is_some());
    }

    /// The task's cancel token, for the work to check.
    pub fn cancel_token(&self) -> CancelToken {
        self.shared.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.shared.cancel.is_cancelled()
    }

    /// Ends the task (as dropping it does).
    pub fn finish(self) {}
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.remove(self.shared.id);
        }
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.shared.snapshot(), f)
    }
}

impl Shared {
    fn snapshot(&self) -> TaskSnapshot {
        let state = lock(&self.state).clone();
        TaskSnapshot {
            id: self.id,
            title: self.title.clone(),
            text: state.text,
            details: state.details,
            fraction: state.fraction,
            cancellable: self.cancellable,
            stopping: self.cancel.is_cancelled(),
            project: self.project.clone(),
            started: self.started,
        }
    }
}

/// A task as it is at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub title: String,
    pub text: String,
    pub details: String,
    /// How much is done, from 0 to 1; `None` while indeterminate.
    pub fraction: Option<f64>,
    pub cancellable: bool,
    /// Cancelled, the work not ended yet.
    pub stopping: bool,
    /// The project the task belongs to; `None` for every window.
    pub project: Option<PathBuf>,
    pub started: Instant,
}

impl TaskSnapshot {
    /// The task has run long enough to be shown at `now`.
    pub fn is_shown(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= SHOW_DELAY
    }

    /// How long until the task is shown (zero once it is).
    pub fn until_shown(&self, now: Instant) -> Duration {
        SHOW_DELAY.saturating_sub(now.saturating_duration_since(self.started))
    }

    /// A window of the project at `root` (`None`: no project) shows it.
    pub fn belongs_to(&self, root: Option<&Path>) -> bool {
        match &self.project {
            None => true,
            Some(p) => root == Some(p.as_path()),
        }
    }

    /// The bar is drawn indeterminate: no fraction, or none done yet (the
    /// reference's `isPaintingIndeterminate`).
    pub fn is_indeterminate(&self) -> bool {
        self.fraction.is_none_or(|f| f == 0.0)
    }
}

/// Stops the observer's waker when dropped.
pub struct Subscription {
    subscriber: Arc<Subscriber>,
    registry: Weak<Registry>,
}

impl Subscription {
    /// The observer is about to read the tasks: the next change wakes it
    /// again.
    pub fn acknowledge(&self) {
        self.subscriber.pending.store(false, Ordering::Release);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            lock(&registry.subscribers).retain(|s| !Arc::ptr_eq(s, &self.subscriber));
        }
    }
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("pending", &self.subscriber.pending.load(Ordering::Relaxed))
            .finish()
    }
}

/// Cancellation of a piece of work: cloned into the work, set once.
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    cancelled: AtomicBool,
    callbacks: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Sets the token and runs the callbacks (once).
    pub fn cancel(&self) {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let callbacks = std::mem::take(&mut *lock(&self.inner.callbacks));
        for f in callbacks {
            f();
        }
    }

    /// Runs `f` when the token is set (at once if it is), to forward the
    /// cancellation to the work's own mechanism (a process to kill, a flag
    /// of its own).
    pub fn on_cancel(&self, f: impl FnOnce() + Send + 'static) {
        {
            let mut callbacks = lock(&self.inner.callbacks);
            if !self.is_cancelled() {
                callbacks.push(Box::new(f));
                return;
            }
        }
        f();
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CancelToken")
            .field(&self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests;
