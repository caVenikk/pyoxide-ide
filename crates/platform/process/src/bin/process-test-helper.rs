//! A small program for the tests of the `process` crate, so they need
//! neither Python nor shell tools. Not shipped.
//!
//! `process-test-helper <mode> [args]`:
//! - `split`: `out-1` and `out-2` on stdout, `err-1` and `err-2` on stderr;
//! - `partial`: `Name: ` without a line break, then waits;
//! - `prompt`: `Name: `, reads a line, prints `Hello, <line>!`;
//! - `echo`: copies stdin to stdout until end of file, then prints `EOF`;
//! - `exit <code>`: exits with the code;
//! - `env <name>`: prints the variable's value or `<unset>`;
//! - `cwd`: prints the working directory;
//! - `sleep`: prints `ready`, sleeps (default Ctrl+C / `SIGINT` handling);
//! - `ignore-interrupt`: ignores Ctrl+C / `SIGINT`, prints `ready`, sleeps;
//! - `tree [detached]`: starts `sleep` as a child, prints `child <pid>` and
//!   `ready`, sleeps; `detached` puts the child in its own process group
//!   (Unix) so only a process tree walk finds it;
//! - `orphan`: starts `sleep` as a child sharing stdout and exits at once;
//! - `flood <bytes>`: writes that many bytes to stdout and exits.

// The children of `tree` and `orphan` are left running on purpose: the
// tests check that killing the tree ends them, or kill them themselves.
#![allow(clippy::zombie_processes)]

use std::io::{self, BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");
    let out = io::stdout();
    let mut out = out.lock();
    match mode {
        "split" => {
            writeln!(out, "out-1").unwrap();
            out.flush().unwrap();
            eprintln!("err-1");
            writeln!(out, "out-2").unwrap();
            out.flush().unwrap();
            eprintln!("err-2");
        }
        "partial" => {
            write!(out, "Name: ").unwrap();
            out.flush().unwrap();
            sleep_long();
        }
        "prompt" => {
            write!(out, "Name: ").unwrap();
            out.flush().unwrap();
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).unwrap();
            writeln!(out, "Hello, {}!", line.trim_end()).unwrap();
        }
        "echo" => {
            let mut buf = [0u8; 4096];
            let mut stdin = io::stdin().lock();
            loop {
                let n = stdin.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).unwrap();
                out.flush().unwrap();
            }
            writeln!(out, "EOF").unwrap();
        }
        "exit" => {
            let code = args.get(1).and_then(|c| c.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        "env" => {
            let name = args.get(1).map(String::as_str).unwrap_or("");
            match std::env::var(name) {
                Ok(value) => writeln!(out, "{value}").unwrap(),
                Err(_) => writeln!(out, "<unset>").unwrap(),
            }
        }
        "cwd" => {
            writeln!(out, "{}", std::env::current_dir().unwrap().display()).unwrap();
        }
        "sleep" => {
            writeln!(out, "ready").unwrap();
            out.flush().unwrap();
            sleep_long();
        }
        "ignore-interrupt" => {
            ignore_interrupt();
            writeln!(out, "ready").unwrap();
            out.flush().unwrap();
            sleep_long();
        }
        "tree" => {
            let detached = args.get(1).map(String::as_str) == Some("detached");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .arg("sleep")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if detached {
                detach(&mut command);
            }
            let child = command.spawn().unwrap();
            writeln!(out, "child {}", child.id()).unwrap();
            writeln!(out, "ready").unwrap();
            out.flush().unwrap();
            sleep_long();
        }
        "orphan" => {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("sleep")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            writeln!(out, "child {}", child.id()).unwrap();
            out.flush().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        }
        "flood" => {
            let mut left: usize = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 65536];
            while left > 0 {
                let n = left.min(chunk.len());
                out.write_all(&chunk[..n]).unwrap();
                left -= n;
            }
            out.flush().unwrap();
        }
        _ => {
            eprintln!("unknown mode {mode:?}");
            std::process::exit(2);
        }
    }
}

fn sleep_long() {
    std::thread::sleep(Duration::from_secs(60));
}

#[cfg(unix)]
fn ignore_interrupt() {
    // SAFETY: sets the disposition of SIGINT to "ignore".
    unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
}

#[cfg(windows)]
fn ignore_interrupt() {
    // SAFETY: sets this process's "ignore Ctrl+C" flag.
    unsafe { windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 1) };
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(_command: &mut Command) {}
