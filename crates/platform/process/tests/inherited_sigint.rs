//! A parent that ignores `SIGINT` (the application started as a background
//! job of a non-interactive shell, a launcher) must not pass the setting on:
//! an ignored signal stays ignored across `exec`, so Stop could not interrupt
//! the program, and Python does not install its `KeyboardInterrupt` handler
//! when `SIGINT` is ignored at startup. A test program of its own: the
//! disposition is process-wide.

#![cfg(unix)]

use std::time::{Duration, Instant};

use process::{CommandLine, ProcessEvent, spawn};

const HELPER: &str = env!("CARGO_BIN_EXE_process-test-helper");

#[test]
fn interrupt_stops_a_process_started_while_sigint_is_ignored() {
    // SAFETY: sets the disposition of SIGINT of this test program to
    // "ignore"; no handler runs.
    unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };

    let (handle, rx) = spawn(&CommandLine::new(HELPER).arg("sleep")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stdout = Vec::new();
    while !String::from_utf8_lossy(&stdout).contains("ready") {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ProcessEvent::Stdout(b)) => stdout.extend(b),
            other => panic!("unexpected {other:?}"),
        }
    }
    handle.interrupt().unwrap();
    let exit = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ProcessEvent::Exited(status)) => break status,
            Ok(_) => {}
            Err(e) => {
                // Do not leave the program running for a minute.
                let _ = handle.kill();
                panic!("the interrupt did not end the process: {e:?}");
            }
        }
    };
    assert_eq!(exit.signal, Some(2));
}
