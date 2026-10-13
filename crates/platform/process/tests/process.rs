//! The process runner against a small helper program built with the crate
//! (`src/bin/process-test-helper.rs`), the same on every OS.

use std::time::{Duration, Instant};

use process::{
    CommandLine, ExitStatus, InterruptHelper, ProcessEvent, ProcessHandle, Receiver, spawn,
};

const HELPER: &str = env!("CARGO_BIN_EXE_process-test-helper");
const TIMEOUT: Duration = Duration::from_secs(20);

fn helper(args: &[&str]) -> CommandLine {
    CommandLine::new(HELPER).args(args)
}

/// The helper program sends Ctrl+C on Windows, as the application does with
/// itself (see `process::set_interrupt_helper`).
fn use_interrupt_helper() {
    process::set_interrupt_helper(Some(InterruptHelper::new(HELPER)));
    os::enable_ctrl_c_in_children();
}

#[derive(Default, Debug)]
struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit: Option<ExitStatus>,
}

impl Output {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout).replace("\r\n", "\n")
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr).replace("\r\n", "\n")
    }

    fn take(&mut self, event: ProcessEvent) {
        match event {
            ProcessEvent::Stdout(b) => self.stdout.extend(b),
            ProcessEvent::Stderr(b) => self.stderr.extend(b),
            ProcessEvent::Exited(s) => {
                assert!(self.exit.is_none(), "two Exited events");
                self.exit = Some(s);
            }
        }
    }
}

/// Reads events until `done` holds or the time is up.
fn read_until(rx: &Receiver<ProcessEvent>, out: &mut Output, done: impl Fn(&Output) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(out) {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(event) => out.take(event),
            Err(e) => panic!("no event ({e:?}); so far {out:?}"),
        }
    }
}

fn run_to_end(rx: &Receiver<ProcessEvent>) -> Output {
    let mut out = Output::default();
    read_until(rx, &mut out, |o| o.exit.is_some());
    // Exited is the last event.
    match rx.recv_timeout(TIMEOUT) {
        Err(flume::RecvTimeoutError::Disconnected) => {}
        other => panic!("event after Exited: {other:?}"),
    }
    out
}

fn wait_stdout(rx: &Receiver<ProcessEvent>, out: &mut Output, needle: &str) {
    read_until(rx, out, |o| o.stdout().contains(needle));
}

fn child_pid(out: &Output) -> u32 {
    let text = out.stdout();
    let line = text.lines().find(|l| l.starts_with("child ")).unwrap();
    line["child ".len()..].trim().parse().unwrap()
}

fn assert_gone(pid: u32) {
    let deadline = Instant::now() + TIMEOUT;
    while os::alive(pid) {
        assert!(Instant::now() < deadline, "process {pid} still alive");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stdout_and_stderr_arrive_separately_and_in_order() {
    let (handle, rx) = spawn(&helper(&["split"])).unwrap();
    let out = run_to_end(&rx);
    assert_eq!(out.stdout(), "out-1\nout-2\n");
    assert_eq!(out.stderr(), "err-1\nerr-2\n");
    let exit = out.exit.unwrap();
    assert!(exit.success());
    assert_eq!(handle.exit_status(), Some(exit));
    assert!(!handle.is_running());
}

#[test]
fn exit_codes() {
    for code in [0, 1, 3, 42] {
        let (_, rx) = spawn(&helper(&["exit", &code.to_string()])).unwrap();
        let exit = run_to_end(&rx).exit.unwrap();
        assert_eq!(exit.code, Some(code));
        assert_eq!(exit.exit_code_text(), code.to_string());
        assert_eq!(
            exit.finished_message(),
            format!("Process finished with exit code {code}")
        );
    }
}

#[test]
fn a_prompt_without_line_break_arrives_while_running() {
    let (handle, rx) = spawn(&helper(&["partial"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "Name: ");
    assert_eq!(out.stdout(), "Name: ");
    assert!(handle.is_running());
    handle.kill().unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
}

#[test]
fn standard_input_answers_a_prompt() {
    let (handle, rx) = spawn(&helper(&["prompt"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "Name: ");
    handle.write_stdin(b"World\n").unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    assert_eq!(out.stdout(), "Name: Hello, World!\n");
    assert!(out.exit.unwrap().success());
}

#[test]
fn standard_input_is_echoed_and_closed() {
    let (handle, rx) = spawn(&helper(&["echo"])).unwrap();
    handle.write_stdin(b"abc").unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "abc");
    handle.write_stdin(b"def\n").unwrap();
    handle.close_stdin();
    assert!(handle.write_stdin(b"late").is_err());
    read_until(&rx, &mut out, |o| o.exit.is_some());
    assert_eq!(out.stdout(), "abcdef\nEOF\n");
}

#[test]
fn environment_and_working_directory() {
    let cmd = helper(&["env", "PYOXIDE_TEST_VALUE"]).with_env("PYOXIDE_TEST_VALUE", "a b");
    let (_, rx) = spawn(&cmd).unwrap();
    assert_eq!(run_to_end(&rx).stdout(), "a b\n");

    let cmd = helper(&["env", "PYOXIDE_TEST_VALUE"])
        .with_env("PYOXIDE_TEST_VALUE", "first")
        .without_env("PYOXIDE_TEST_VALUE");
    let (_, rx) = spawn(&cmd).unwrap();
    assert_eq!(run_to_end(&rx).stdout(), "<unset>\n");

    // Without the system environment, PATH is not passed on.
    let mut cmd = helper(&["env", "PATH"]).with_env("PYOXIDE_TEST_VALUE", "x");
    cmd.inherit_env = false;
    let (_, rx) = spawn(&cmd).unwrap();
    assert_eq!(run_to_end(&rx).stdout(), "<unset>\n");

    let dir = std::env::temp_dir().canonicalize().unwrap();
    let (_, rx) = spawn(&helper(&["cwd"]).current_dir(&dir)).unwrap();
    let printed = std::path::PathBuf::from(run_to_end(&rx).stdout().trim_end());
    assert_eq!(printed.canonicalize().unwrap(), dir);
}

#[test]
fn a_missing_program_is_an_error() {
    let err = spawn(&CommandLine::new("pyoxide-no-such-program-here")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

/// The reference's soft stop ends a process that keeps the default Ctrl+C
/// handling: `SIGINT` on Unix, `STATUS_CONTROL_C_EXIT` on Windows.
#[test]
fn interrupt_stops_a_process() {
    use_interrupt_helper();
    let (handle, rx) = spawn(&helper(&["sleep"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "ready");
    handle.interrupt().unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    let exit = out.exit.unwrap();
    if cfg!(windows) {
        assert_eq!(exit.code, Some(0xC000_013Au32 as i32));
        assert_eq!(
            exit.exit_code_text(),
            "-1073741510 (0xC000013A: interrupted by Ctrl+C)"
        );
    } else {
        assert_eq!(exit.signal, Some(2));
        assert_eq!(exit.exit_code(), 130);
        assert_eq!(
            exit.exit_code_text(),
            "130 (interrupted by signal 2:SIGINT)"
        );
    }
}

/// First stop interrupts, the second kills a process that ignores the
/// interrupt.
#[test]
fn stop_twice_kills_a_process_that_ignores_interrupts() {
    use_interrupt_helper();
    let (handle, rx) = spawn(&helper(&["ignore-interrupt"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "ready");
    assert!(!handle.is_stopping());
    handle.stop().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(handle.is_running(), "the interrupt was not ignored");
    assert!(handle.is_stopping());
    handle.stop().unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    let exit = out.exit.unwrap();
    if cfg!(windows) {
        assert_eq!(exit.code, Some(-1));
    } else {
        assert_eq!(exit.signal, Some(9));
        assert_eq!(
            exit.exit_code_text(),
            "137 (interrupted by signal 9:SIGKILL)"
        );
    }
    assert!(!handle.is_stopping());
    // Stopping an ended process does nothing.
    handle.stop().unwrap();
    handle.kill().unwrap();
    handle.interrupt().unwrap();
}

fn kill_tree(args: &[&str]) {
    let (handle, rx) = spawn(&helper(args)).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "ready");
    let grandchild = child_pid(&out);
    assert!(os::alive(grandchild));
    handle.kill().unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    assert_gone(grandchild);
}

#[test]
fn kill_ends_the_whole_tree() {
    kill_tree(&["tree"]);
}

/// A descendant in another process group is found by the process tree walk
/// (Unix; on Windows the same as above).
#[test]
fn kill_ends_a_descendant_outside_the_process_group() {
    kill_tree(&["tree", "detached"]);
}

#[test]
fn interrupt_reaches_a_descendant_outside_the_process_group() {
    use_interrupt_helper();
    let (handle, rx) = spawn(&helper(&["tree", "detached"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "ready");
    let grandchild = child_pid(&out);
    handle.interrupt().unwrap();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    assert_gone(grandchild);
}

/// A descendant that keeps standard output open does not hold back the end
/// of the process (the reference stops reading when the process exits).
#[test]
fn an_orphan_holding_the_pipe_does_not_delay_exited() {
    let started = Instant::now();
    let (_, rx) = spawn(&helper(&["orphan"])).unwrap();
    let mut out = Output::default();
    read_until(&rx, &mut out, |o| o.exit.is_some());
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(out.exit.unwrap().success());
    let grandchild = child_pid(&out);
    os::kill(grandchild);
}

/// A process printing faster than the consumer reads is throttled (the
/// bounded channel and the pipe fill up) and nothing is lost.
#[test]
fn backpressure_throttles_a_flood_without_losing_output() {
    const SIZE: usize = 32 * 1024 * 1024;
    let (handle, rx) = spawn(&helper(&["flood", &SIZE.to_string()])).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(handle.is_running(), "the flood was not throttled");
    assert!(rx.len() <= process::EVENT_CHANNEL_CAPACITY);
    let mut total = 0usize;
    let mut largest = 0usize;
    let exit = loop {
        match rx.recv_timeout(TIMEOUT).unwrap() {
            ProcessEvent::Stdout(b) => {
                assert!(b.iter().all(|c| *c == b'x'));
                largest = largest.max(b.len());
                total += b.len();
            }
            ProcessEvent::Stderr(b) => panic!("stderr: {}", String::from_utf8_lossy(&b)),
            ProcessEvent::Exited(s) => break s,
        }
    };
    assert_eq!(total, SIZE);
    assert!(largest <= process::READ_CHUNK_SIZE);
    assert!(exit.success());
}

#[test]
fn dropping_the_handle_does_not_stop_the_process() {
    let (handle, rx) = spawn(&helper(&["sleep"])).unwrap();
    let mut out = Output::default();
    wait_stdout(&rx, &mut out, "ready");
    let pid = handle.pid();
    let probe: ProcessHandle = handle.clone();
    drop(handle);
    drop(rx);
    std::thread::sleep(Duration::from_millis(200));
    assert!(probe.is_running());
    drop(probe);
    assert!(os::alive(pid));
    os::kill(pid);
    assert_gone(pid);
}

#[cfg(unix)]
mod os {
    pub fn enable_ctrl_c_in_children() {}

    pub fn alive(pid: u32) -> bool {
        let pid = pid as libc::pid_t;
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        // A zombie waiting for its new parent is dead.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rfind(')')
                .and_then(|i| stat[i + 1..].split_whitespace().next())
                .is_some_and(|state| state != "Z"),
            Err(_) => !cfg!(target_os = "linux"),
        }
    }

    pub fn kill(pid: u32) {
        // SAFETY: plain syscall on a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, STILL_ACTIVE};
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    };

    /// Children inherit "ignore Ctrl+C" from this process; a test runner
    /// may have been started with it (a new process group). The IDE is
    /// started normally, so its children handle Ctrl+C.
    pub fn enable_ctrl_c_in_children() {
        // SAFETY: restores normal Ctrl+C processing of this process only; it
        // never attaches to a child's console, so no event reaches it.
        unsafe { SetConsoleCtrlHandler(None, FALSE) };
    }

    pub fn alive(pid: u32) -> bool {
        // SAFETY: a handle opened, queried and closed here.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }

    pub fn kill(pid: u32) {
        // SAFETY: a handle opened, used and closed here.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}
