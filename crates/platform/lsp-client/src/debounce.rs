//! Work that runs a while after the last request for it: pulling the
//! diagnostics of a document 300 ms after its last edit, as the
//! reference's highlighting pass does. One thread per debouncer.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

struct State<K, V> {
    due: HashMap<K, (Instant, V)>,
    closed: bool,
}

struct Shared<K, V> {
    state: Mutex<State<K, V>>,
    changed: Condvar,
}

impl<K, V> Shared<K, V> {
    fn lock(&self) -> MutexGuard<'_, State<K, V>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs `fire(key, value)` on its thread once `delay` has passed since the
/// last [`Debouncer::schedule`] of `key`.
pub(crate) struct Debouncer<K, V> {
    shared: Arc<Shared<K, V>>,
}

impl<K, V> Debouncer<K, V>
where
    K: Hash + Eq + Clone + Send + 'static,
    V: Send + 'static,
{
    pub fn new(name: &str, fire: impl Fn(K, V) + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                due: HashMap::new(),
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let worker = shared.clone();
        // Without the thread nothing is pulled; the rest still works.
        let _ = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run(&worker, fire));
        Self { shared }
    }

    /// (Re)starts the delay of `key`; `value` replaces an earlier one.
    pub fn schedule(&self, key: K, value: V, delay: Duration) {
        let mut state = self.shared.lock();
        state.due.insert(key, (Instant::now() + delay, value));
        self.shared.changed.notify_all();
    }

    /// Forgets a scheduled key.
    pub fn cancel(&self, key: &K) {
        self.shared.lock().due.remove(key);
    }
}

impl<K, V> Drop for Debouncer<K, V> {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
    }
}

fn run<K: Hash + Eq + Clone, V>(shared: &Shared<K, V>, fire: impl Fn(K, V)) {
    let mut state = shared.lock();
    loop {
        if state.closed {
            return;
        }
        let now = Instant::now();
        let due: Vec<K> = state
            .due
            .iter()
            .filter(|(_, (at, _))| *at <= now)
            .map(|(k, _)| k.clone())
            .collect();
        if !due.is_empty() {
            let ready: Vec<(K, V)> = due
                .into_iter()
                .filter_map(|k| state.due.remove(&k).map(|(_, v)| (k, v)))
                .collect();
            drop(state);
            for (key, value) in ready {
                fire(key, value);
            }
            state = shared.lock();
            continue;
        }
        let next = state.due.values().map(|(at, _)| *at).min();
        state = match next {
            Some(at) => {
                shared
                    .changed
                    .wait_timeout(state, at.saturating_duration_since(now))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_schedule_wins() {
        let (tx, rx) = flume::unbounded();
        let debouncer = Debouncer::new("test debounce", move |key: &'static str, value: u32| {
            let _ = tx.send((key, value, Instant::now()));
        });
        // "a" is due far in the future, so no scheduler delay of a busy
        // machine can fire it before it is rescheduled below.
        debouncer.schedule("a", 1, Duration::from_secs(60));
        debouncer.schedule("b", 2, Duration::from_millis(10));
        let (key, value, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((key, value), ("b", 2));
        let rescheduled = Instant::now();
        debouncer.schedule("a", 3, Duration::from_millis(50));
        let (key, value, at) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((key, value), ("a", 3));
        assert!(
            at - rescheduled >= Duration::from_millis(50),
            "{:?}",
            at - rescheduled
        );
        debouncer.schedule("c", 4, Duration::from_millis(300));
        debouncer.cancel(&"c");
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        debouncer.schedule("d", 5, Duration::ZERO);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().1, 5);
    }
}
