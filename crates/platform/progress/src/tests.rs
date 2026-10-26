use super::*;
use std::sync::atomic::AtomicUsize;

#[test]
fn a_task_lives_until_dropped() {
    let tasks = Tasks::new();
    let task = tasks.task("Scanning Files").text("Scanning files…").start();
    let snap = tasks.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].title, "Scanning Files");
    assert_eq!(snap[0].text, "Scanning files…");
    assert!(snap[0].is_indeterminate());
    assert!(!snap[0].cancellable);
    drop(task);
    assert!(tasks.is_empty());
}

#[test]
fn updates_from_another_thread() {
    let tasks = Tasks::new();
    let task = Arc::new(tasks.task("Indexing").start());
    let t = task.clone();
    std::thread::spawn(move || {
        t.set_text("Analyzing project…");
        t.set_details("src/app");
        t.set_fraction(0.25);
    })
    .join()
    .unwrap();
    let snap = &tasks.snapshot()[0];
    assert_eq!(snap.text, "Analyzing project…");
    assert_eq!(snap.details, "src/app");
    assert_eq!(snap.fraction, Some(0.25));
    assert!(!snap.is_indeterminate());
    task.set_fraction(7.0);
    assert_eq!(tasks.snapshot()[0].fraction, Some(1.0));
    task.set_indeterminate();
    assert_eq!(tasks.snapshot()[0].fraction, None);
}

#[test]
fn zero_done_paints_indeterminate() {
    let tasks = Tasks::new();
    let task = tasks.task("x").start();
    task.set_fraction(0.0);
    assert!(tasks.snapshot()[0].is_indeterminate());
}

#[test]
fn snapshot_keeps_the_start_order() {
    let tasks = Tasks::new();
    let a = tasks.task("a").start();
    let b = tasks.task("b").start();
    let c = tasks.task("c").start();
    drop(b);
    let titles: Vec<_> = tasks.snapshot().into_iter().map(|t| t.title).collect();
    assert_eq!(titles, ["a", "c"]);
    assert!(a.id() < c.id());
}

#[test]
fn waker_coalesces_until_acknowledged() {
    let tasks = Tasks::new();
    let woken = Arc::new(AtomicUsize::new(0));
    let w = woken.clone();
    let sub = tasks.subscribe(move || {
        w.fetch_add(1, Ordering::SeqCst);
    });
    let task = tasks.task("t").start();
    task.set_text("one");
    task.set_text("two");
    task.set_fraction(0.5);
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    sub.acknowledge();
    // An unchanged value is no change.
    task.set_text("two");
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    task.set_details("d");
    assert_eq!(woken.load(Ordering::SeqCst), 2);
    sub.acknowledge();
    drop(task);
    assert_eq!(woken.load(Ordering::SeqCst), 3);
    sub.acknowledge();
    drop(sub);
    let _t = tasks.task("u").start();
    assert_eq!(woken.load(Ordering::SeqCst), 3);
}

#[test]
fn cancel_sets_the_token_and_runs_callbacks() {
    let tasks = Tasks::new();
    let task = tasks.task("Fetching…").cancellable().start();
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    task.cancel_token()
        .on_cancel(move || f.store(true, Ordering::SeqCst));
    assert!(tasks.cancel(task.id()));
    assert!(task.is_cancelled());
    assert!(flag.load(Ordering::SeqCst));
    let snap = &tasks.snapshot()[0];
    assert!(snap.stopping);
    // Once is enough.
    assert!(!tasks.cancel(task.id()));
    // A callback added afterwards runs at once.
    let late = Arc::new(AtomicBool::new(false));
    let l = late.clone();
    task.cancel_token()
        .on_cancel(move || l.store(true, Ordering::SeqCst));
    assert!(late.load(Ordering::SeqCst));
}

#[test]
fn a_task_without_cancel_button_is_not_cancelled() {
    let tasks = Tasks::new();
    let task = tasks.task("Starting ty…").start();
    assert!(!tasks.cancel(task.id()));
    assert!(!task.is_cancelled());
    assert!(!tasks.cancel(TaskId(999)));
}

#[test]
fn the_works_own_token() {
    let tasks = Tasks::new();
    let token = CancelToken::new();
    let task = tasks
        .task("Pushing…")
        .cancellable()
        .cancel_token(token.clone())
        .start();
    tasks.cancel(task.id());
    assert!(token.is_cancelled());
}

#[test]
fn shown_after_the_delay() {
    let tasks = Tasks::new();
    let _task = tasks.task("t").start();
    let snap = &tasks.snapshot()[0];
    assert!(!snap.is_shown(snap.started));
    assert_eq!(snap.until_shown(snap.started), SHOW_DELAY);
    let later = snap.started + SHOW_DELAY;
    assert!(snap.is_shown(later));
    assert_eq!(snap.until_shown(later), Duration::ZERO);
}

#[test]
fn project_tasks_belong_to_their_windows() {
    let tasks = Tasks::new();
    let _a = tasks.task("a").project("/p").start();
    let _b = tasks.task("b").start();
    let snap = tasks.snapshot();
    assert!(snap[0].belongs_to(Some(Path::new("/p"))));
    assert!(!snap[0].belongs_to(Some(Path::new("/q"))));
    assert!(!snap[0].belongs_to(None));
    assert!(snap[1].belongs_to(None));
    assert!(snap[1].belongs_to(Some(Path::new("/q"))));
}

#[test]
fn a_task_outliving_its_registry() {
    let tasks = Tasks::new();
    let task = tasks.task("t").start();
    drop(tasks);
    task.set_text("still fine");
    drop(task);
}

#[test]
fn pausing_holds_the_work_until_resumed() {
    let tasks = Tasks::new();
    let task = tasks
        .task("Scanning files…")
        .pausable("Scanning paused")
        .start();
    let token = task.pause_token().unwrap();
    assert!(!tasks.snapshot()[0].paused);
    assert!(tasks.toggle_pause(task.id()));
    let snap = &tasks.snapshot()[0];
    assert!(snap.pausable && snap.paused);
    assert_eq!(snap.paused_text, "Scanning paused");
    let passed = Arc::new(AtomicBool::new(false));
    let worker = {
        let passed = passed.clone();
        let token = token.clone();
        std::thread::spawn(move || {
            token.wait_while_paused(&|| false);
            passed.store(true, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    assert!(!passed.load(Ordering::SeqCst));
    assert!(tasks.toggle_pause(task.id()));
    worker.join().unwrap();
    assert!(passed.load(Ordering::SeqCst));
    assert!(!tasks.snapshot()[0].paused);
}

#[test]
fn a_paused_wait_ends_when_cancelled() {
    let token = PauseToken::new();
    token.pause();
    let cancel = CancelToken::new();
    let c = cancel.clone();
    let worker = std::thread::spawn(move || token.wait_while_paused(&|| c.is_cancelled()));
    cancel.cancel();
    worker.join().unwrap();
}

#[test]
fn a_task_without_pause_button_is_not_paused() {
    let tasks = Tasks::new();
    let task = tasks.task("Updating").start();
    assert!(task.pause_token().is_none());
    assert!(!tasks.toggle_pause(task.id()));
    assert!(!tasks.snapshot()[0].pausable);
}
