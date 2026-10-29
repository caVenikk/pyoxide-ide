//! What a window started and ends with it: the parts of a window that
//! start processes or work (run and debug sessions, terminal sessions,
//! background tasks) register here, so that closing the window ends all of
//! them, and the processes that run are asked about first instead of being
//! dropped silently (the reference's `BaseContentCloseListener.canClose`
//! of every run content, with `TerminateRemoteProcessDialog`).
//!
//! A part registers with [`register`] (or [`register_fn`]) and keeps the
//! [`Registration`]: it is part of the window as long as the registration
//! lives. [`running`] lists what runs, to ask about each before the window
//! closes; [`end`] ends every part and closes the window's dialogs and
//! popups (modal and non-modal, `dialog_window`), with what is open over
//! them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, Global, Task};

/// What happens to a running process when its window closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// It ends, with its children.
    Terminate,
    /// It goes on running, without the IDE (a debug session detaches).
    Disconnect,
}

/// Ends a running process or leaves it ([`Running::stop`]).
type StopFn = dyn Fn(Stop, &mut App) -> Task<()>;

/// A process that runs in a window, asked about before the window closes.
#[derive(Clone)]
pub struct Running {
    name: String,
    stop: Rc<StopFn>,
}

impl Running {
    /// A process named `name` (a run configuration's or a debug session's
    /// name) that `stop` ends or leaves; the task `stop` returns is done
    /// when it ended or was left.
    pub fn new(
        name: impl Into<String>,
        stop: impl Fn(Stop, &mut App) -> Task<()> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            stop: Rc::new(stop),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ends the process or leaves it running; the task is done when it is.
    pub fn stop(&self, how: Stop, cx: &mut App) -> Task<()> {
        (self.stop)(how, cx)
    }
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").field("name", &self.name).finish()
    }
}

/// One part of a window that ends with it.
pub trait Ending: 'static {
    /// The processes that run now and are asked about before they end;
    /// empty when nothing runs or it ends silently.
    fn running(&self, cx: &App) -> Vec<Running> {
        let _ = cx;
        Vec::new()
    }

    /// Ends everything that is left (processes with their children, work
    /// in progress) without asking; the task is done when it ended.
    fn end(&self, cx: &mut App) -> Task<()>;
}

/// An [`Ending`] made of two closures.
struct FnEnding<R, E> {
    running: R,
    end: E,
}

impl<R, E> Ending for FnEnding<R, E>
where
    R: Fn(&App) -> Vec<Running> + 'static,
    E: Fn(&mut App) -> Task<()> + 'static,
{
    fn running(&self, cx: &App) -> Vec<Running> {
        (self.running)(cx)
    }

    fn end(&self, cx: &mut App) -> Task<()> {
        (self.end)(cx)
    }
}

/// A part's place in its window: dropped, the part no longer ends with the
/// window.
#[must_use = "the part ends with the window only while the registration lives"]
pub struct Registration {
    _ending: Rc<dyn Ending>,
}

#[derive(Default)]
struct Registry {
    by_window: RefCell<HashMap<AnyWindowHandle, Vec<Weak<dyn Ending>>>>,
}

impl Global for Registry {}

/// Registers a part of `window` that ends with it.
pub fn register(window: AnyWindowHandle, ending: impl Ending, cx: &mut App) -> Registration {
    let ending: Rc<dyn Ending> = Rc::new(ending);
    let registry = cx.default_global::<Registry>();
    let mut by_window = registry.by_window.borrow_mut();
    let parts = by_window.entry(window).or_default();
    parts.retain(|p| p.strong_count() > 0);
    parts.push(Rc::downgrade(&ending));
    Registration { _ending: ending }
}

/// [`register`] with closures: `running` lists what runs and is asked
/// about, `end` ends everything.
pub fn register_fn(
    window: AnyWindowHandle,
    running: impl Fn(&App) -> Vec<Running> + 'static,
    end: impl Fn(&mut App) -> Task<()> + 'static,
    cx: &mut App,
) -> Registration {
    register(window, FnEnding { running, end }, cx)
}

/// The live parts of `window`, in the order they registered.
fn parts(window: AnyWindowHandle, cx: &App) -> Vec<Rc<dyn Ending>> {
    cx.try_global::<Registry>()
        .and_then(|r| {
            r.by_window
                .borrow()
                .get(&window)
                .map(|parts| parts.iter().filter_map(Weak::upgrade).collect())
        })
        .unwrap_or_default()
}

/// What runs in `window` and is asked about before it closes.
pub fn running(window: AnyWindowHandle, cx: &App) -> Vec<Running> {
    parts(window, cx)
        .iter()
        .flat_map(|p| p.running(cx))
        .collect()
}

/// Closes the dialogs and popups of `window` and ends every part of it;
/// the tasks are done when the parts ended. The window itself is the
/// caller's to close, after them.
pub fn end(window: AnyWindowHandle, cx: &mut App) -> Vec<Task<()>> {
    if let Some(dialog) = crate::dialog_window::modal_of(window, cx) {
        crate::dialog_window::close(dialog, cx);
    }
    for dialog in crate::dialog_window::non_modal_of(window, cx) {
        crate::dialog_window::close(dialog, cx);
    }
    let parts = parts(window, cx);
    if let Some(registry) = cx.try_global::<Registry>() {
        registry.by_window.borrow_mut().remove(&window);
    }
    parts.iter().map(|p| p.end(cx)).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{AppContext as _, Empty, TestAppContext};

    use super::*;

    fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Empty))
                .unwrap()
                .into()
        })
    }

    #[gpui::test]
    fn parts_end_with_their_window_while_registered(cx: &mut TestAppContext) {
        let a = window(cx);
        let b = window(cx);
        let ended = Rc::new(Cell::new(0));
        let stopped = Rc::new(RefCell::new(Vec::new()));
        let (kept, dropped, other) = cx.update(|cx| {
            let stops = stopped.clone();
            let kept = register_fn(
                a,
                move |_| {
                    let stops = stops.clone();
                    vec![Running::new("main", move |how, _| {
                        stops.borrow_mut().push(how);
                        Task::ready(())
                    })]
                },
                {
                    let ended = ended.clone();
                    move |_| {
                        ended.set(ended.get() + 1);
                        Task::ready(())
                    }
                },
                cx,
            );
            let counter = |ended: &Rc<Cell<u32>>| {
                let ended = ended.clone();
                move |_: &mut App| {
                    ended.set(ended.get() + 10);
                    Task::ready(())
                }
            };
            let dropped = register_fn(a, |_| Vec::new(), counter(&ended), cx);
            let other = register_fn(b, |_| Vec::new(), counter(&ended), cx);
            (kept, dropped, other)
        });
        drop(dropped);
        cx.update(|cx| {
            let running = running(a, cx);
            assert_eq!(running.len(), 1);
            assert_eq!(running[0].name(), "main");
            running[0].stop(Stop::Disconnect, cx).detach();
            assert!(super::running(b, cx).is_empty());
            assert_eq!(end(a, cx).len(), 1);
            // Ended once: the window's parts are forgotten.
            assert!(end(a, cx).is_empty());
        });
        assert_eq!(ended.get(), 1, "only the registered part of `a` ended");
        assert_eq!(*stopped.borrow(), [Stop::Disconnect]);
        drop((kept, other));
    }
}
