//! The children of a process (`process::child_processes`), the check a
//! terminal tab makes before it closes: a shell with a running command has
//! a child, an idle one has none.

use std::time::{Duration, Instant};

use process::{CommandLine, ProcessEvent, ProcessHandle, Receiver, spawn};

const HELPER: &str = env!("CARGO_BIN_EXE_process-test-helper");
const TIMEOUT: Duration = Duration::from_secs(20);

/// Starts the helper with `args` and reads its output up to `ready`.
fn start(args: &[&str]) -> (ProcessHandle, Receiver<ProcessEvent>, String) {
    let (handle, rx) = spawn(&CommandLine::new(HELPER).args(args)).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    let mut out = Vec::new();
    while !String::from_utf8_lossy(&out).contains("ready") {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ProcessEvent::Stdout(b)) => out.extend(b),
            other => {
                let _ = handle.kill();
                panic!("the helper did not get ready: {other:?}");
            }
        }
    }
    (handle, rx, String::from_utf8_lossy(&out).into_owned())
}

fn child_of(output: &str) -> u32 {
    let line = output
        .lines()
        .find(|l| l.starts_with("child "))
        .expect("a child line");
    line["child ".len()..].trim().parse().unwrap()
}

/// Waits for the process to end after a kill.
fn finish(handle: &ProcessHandle, rx: &Receiver<ProcessEvent>) {
    let _ = handle.kill();
    let deadline = Instant::now() + TIMEOUT;
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if matches!(event, ProcessEvent::Exited(_)) {
            return;
        }
    }
}

#[test]
fn a_process_without_children_has_none() {
    let (handle, rx, _) = start(&["sleep"]);
    assert!(process::child_processes(handle.pid()).is_empty());
    assert!(!process::has_child_processes(handle.pid()));
    finish(&handle, &rx);
}

#[test]
fn a_started_child_is_found_and_is_gone_with_the_tree() {
    let (handle, rx, output) = start(&["tree"]);
    let child = child_of(&output);
    assert_eq!(process::child_processes(handle.pid()), [child]);
    assert!(process::has_child_processes(handle.pid()));
    // The child itself has no children: only the direct ones are listed.
    assert!(process::child_processes(child).is_empty());
    finish(&handle, &rx);
    let deadline = Instant::now() + TIMEOUT;
    while process::has_child_processes(handle.pid()) {
        assert!(Instant::now() < deadline, "the child is still listed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A child in a process group of its own (a job the shell put in the
/// background) is still the shell's child.
#[cfg(unix)]
#[test]
fn a_child_in_its_own_process_group_is_found() {
    let (handle, rx, output) = start(&["tree", "detached"]);
    assert_eq!(process::child_processes(handle.pid()), [child_of(&output)]);
    // The kill walks the process tree, so it reaches the child too.
    finish(&handle, &rx);
}

#[test]
fn a_process_that_does_not_exist_has_no_children() {
    assert!(process::child_processes(i32::MAX as u32).is_empty());
}
