//! Quitting the application ends what runs in its terminals, also a program
//! that ignores the hang-up and runs in a process group of its own. The
//! helper's `quit-host` mode is the application: it starts a session, ends
//! all sessions the way the quit does and exits without dropping anything.

use std::process::Command;
use std::time::{Duration, Instant};

const HELPER: &str = env!("CARGO_BIN_EXE_terminal-test-helper");

fn wait_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !process::is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn quitting_ends_a_program_that_ignores_the_hangup() {
    let output = Command::new(HELPER)
        .args(["quit-host", "orphan"])
        .output()
        .expect("run the host");
    assert!(output.status.success(), "host failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid: u32 = stdout
        .lines()
        .find_map(|l| l.strip_prefix("host child "))
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or_else(|| panic!("no child pid in {stdout:?}"));
    let ended = wait_gone(pid);
    if !ended {
        process::kill_process(pid);
    }
    assert!(ended, "the program {pid} outlived the application");
}
