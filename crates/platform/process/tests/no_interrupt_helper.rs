//! Without an interrupt helper (a separate test program: the helper is a
//! process-wide setting). On Windows an interrupt then fails and the first
//! Stop kills, as the reference falls back to a hard destroy when it cannot
//! send Ctrl+C; on Unix the interrupt needs no helper.

use std::time::{Duration, Instant};

use process::{CommandLine, ProcessEvent, spawn};

const HELPER: &str = env!("CARGO_BIN_EXE_process-test-helper");

#[test]
fn stop_without_a_helper() {
    let (handle, rx) = spawn(&CommandLine::new(HELPER).arg("sleep")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stdout = Vec::new();
    while !String::from_utf8_lossy(&stdout).contains("ready") {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ProcessEvent::Stdout(b)) => stdout.extend(b),
            other => panic!("unexpected {other:?}"),
        }
    }
    if cfg!(windows) {
        let err = handle.interrupt().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(handle.is_running());
    }
    handle.stop().unwrap();
    let exit = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ProcessEvent::Exited(status)) => break status,
            Ok(_) => {}
            Err(e) => panic!("no exit: {e:?}"),
        }
    };
    if cfg!(windows) {
        assert_eq!(exit.code, Some(-1));
        assert_eq!(
            exit.finished_message(),
            "Process finished with exit code -1"
        );
    } else {
        assert_eq!(exit.signal, Some(2));
    }
}
