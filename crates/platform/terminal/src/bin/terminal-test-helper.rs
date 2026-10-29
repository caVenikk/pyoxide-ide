//! A small program for the PTY tests of the `terminal` crate, so they need
//! no real shell. Not shipped.
//!
//! `terminal-test-helper <mode> [args]`:
//! - `echo`: reads lines, prints `got <line>` for each; `exit` ends it;
//! - `size`: prints `size <columns>x<rows>` of its terminal, again after
//!   every line read; `exit` ends it;
//! - `exit <code>`: exits with the code;
//! - `paste`: raw input, turns bracketed paste on (`ESC [?2004h`), prints
//!   `ready`, reads until Ctrl+D (0x04) and prints `pasted: <bytes>` with
//!   ESC, CR and LF spelled `<ESC>`, `<CR>`, `<LF>`; waits for one more
//!   byte before it exits;
//! - `env <name>...`: prints `<name>=<value>` or `<name>=<unset>`;
//! - `cwd`: prints `cwd <working directory>`;
//! - `osc`: prints a prompt with OSC 133 marks, an OSC 7 working directory,
//!   a title (OSC 2) and an OSC 7701 event, then `done`; waits for a line;
//! - `sleep`: prints `ready` and sleeps;
//! - `child`: starts a program that waits for its input to close as its
//!   child, prints `ready` and sleeps (a shell with a command running);
//! - `wait`: reads its input until it closes (the child of `child`);
//! - `hold`: ignores the hang-up, sleeps (a program a closed terminal does
//!   not end);
//! - `orphan`: starts `hold` in a process group of its own, prints
//!   `child <pid>` and sleeps (a shell with a job that ignores the hang-up);
//! - `quit-host <mode>`: the application at its quit: runs this program in
//!   `<mode>` in a terminal session, waits for its `child <pid>` line, prints
//!   `host child <pid>`, ends all sessions as the application does and exits
//!   at once without dropping anything;
//! - `flood <bytes>`: prints numbered lines worth that many bytes, then
//!   `flood done`; waits for a line.

use std::io::{self, BufRead, Read, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match mode {
        "echo" => {
            for line in io::stdin().lock().lines() {
                let line = line.unwrap_or_default();
                let line = line.trim_end();
                if line == "exit" {
                    break;
                }
                writeln!(out, "got {line}").unwrap();
                out.flush().unwrap();
            }
        }
        "size" => {
            let print = |out: &mut io::StdoutLock| {
                let (columns, rows) = os::size();
                writeln!(out, "size {columns}x{rows}").unwrap();
                out.flush().unwrap();
            };
            print(&mut out);
            for line in io::stdin().lock().lines() {
                if line.unwrap_or_default().trim_end() == "exit" {
                    break;
                }
                print(&mut out);
            }
        }
        "exit" => {
            let code = args.get(1).and_then(|c| c.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        "paste" => {
            os::raw_input();
            write!(out, "\x1b[?2004hready\r\n").unwrap();
            out.flush().unwrap();
            let mut got = Vec::new();
            let mut byte = [0u8; 1];
            let mut stdin = io::stdin().lock();
            while stdin.read(&mut byte).unwrap_or(0) == 1 && byte[0] != 0x04 {
                got.push(byte[0]);
            }
            let text = String::from_utf8_lossy(&got)
                .replace('\x1b', "<ESC>")
                .replace('\r', "<CR>")
                .replace('\n', "<LF>");
            write!(out, "pasted: {text}\r\n").unwrap();
            out.flush().unwrap();
            // Stays alive until one more byte arrives: ConPTY may drop the
            // output of a program that exits right after writing it.
            let _ = stdin.read(&mut byte);
        }
        "env" => {
            for name in &args[1..] {
                match std::env::var(name) {
                    Ok(value) => writeln!(out, "{name}={value}").unwrap(),
                    Err(_) => writeln!(out, "{name}=<unset>").unwrap(),
                }
            }
            out.flush().unwrap();
            wait_line();
        }
        "cwd" => {
            let cwd = std::env::current_dir().unwrap();
            writeln!(out, "cwd {}", cwd.display()).unwrap();
            out.flush().unwrap();
            wait_line();
        }
        "osc" => {
            write!(out, "\x1b]133;A\x07$ \x1b]133;B\x07").unwrap();
            write!(out, "\x1b]133;C\x07output\n\x1b]133;D;3\x07").unwrap();
            write!(out, "\x1b]7;file://localhost/tmp/a%20b\x1b\\").unwrap();
            write!(out, "\x1b]2;helper title\x07").unwrap();
            write!(out, "\x1b]7701;hello\x07").unwrap();
            writeln!(out, "done").unwrap();
            out.flush().unwrap();
            wait_line();
        }
        "sleep" => {
            writeln!(out, "ready").unwrap();
            out.flush().unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "child" => {
            // The child ends when this program does: its input is a pipe
            // this program holds.
            #[allow(clippy::zombie_processes)]
            let _child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("wait")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            writeln!(out, "ready").unwrap();
            out.flush().unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "hold" => {
            os::ignore_hangup();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "orphan" => {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .arg("hold")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            os::own_process_group(&mut command);
            #[allow(clippy::zombie_processes)]
            let child = command.spawn().unwrap();
            writeln!(out, "child {}", child.id()).unwrap();
            out.flush().unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "quit-host" => quit_host(args.get(1).map_or("orphan", String::as_str)),
        "wait" => {
            let mut sink = Vec::new();
            let _ = io::stdin().lock().read_to_end(&mut sink);
        }
        "flood" => {
            let total: usize = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(1 << 20);
            let mut written = 0;
            let mut n = 0u64;
            let mut buf = Vec::with_capacity(1 << 16);
            while written < total {
                buf.clear();
                while buf.len() < (1 << 16) && written + buf.len() < total {
                    n += 1;
                    writeln!(
                        buf,
                        "line {n:08} the quick brown fox jumps over the lazy dog 0123456789"
                    )
                    .unwrap();
                }
                out.write_all(&buf).unwrap();
                written += buf.len();
            }
            writeln!(out, "flood done").unwrap();
            out.flush().unwrap();
            // Alive until a line arrives (see `paste`).
            wait_line();
        }
        _ => {
            eprintln!("unknown mode {mode:?}");
            std::process::exit(2);
        }
    }
}

fn quit_host(mode: &str) {
    use terminal::start::{Os, environment};
    use terminal::{Session, ShellCommand, TerminalSize};

    let options = terminal::SessionOptions {
        command: ShellCommand {
            program: std::env::current_exe().unwrap().display().to_string(),
            args: vec![mode.to_owned()],
        },
        working_directory: std::env::temp_dir(),
        env: environment(Os::current(), &[], &[]),
        size: TerminalSize::new(120, 24),
        scrollback_lines: 0,
    };
    let (session, _events) = Session::spawn(options).expect("start the session");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let child = loop {
        let text = session.snapshot().text();
        let found = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("child "))
            .and_then(|pid| pid.trim().parse::<u32>().ok());
        if let Some(pid) = found {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no child line:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    println!("host child {child}");
    io::stdout().flush().unwrap();
    terminal::end_all_sessions(terminal::QUIT_GRACE);
    // Like the application's quit: nothing is dropped.
    std::process::exit(0);
}

fn wait_line() {
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
}

#[cfg(unix)]
mod os {
    /// A hang-up does not end this process.
    pub fn ignore_hangup() {
        // SAFETY: sets a signal disposition.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    }

    /// The command starts in a process group of its own.
    pub fn own_process_group(command: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    /// Columns and rows of the terminal on standard output.
    pub fn size() -> (u16, u16) {
        // SAFETY: a zeroed winsize filled by the call.
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws);
            (ws.ws_col, ws.ws_row)
        }
    }

    /// Raw input: no line editing, no echo, no signals.
    pub fn raw_input() {
        // SAFETY: termios of standard input, read and written back.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                libc::cfmakeraw(&mut t);
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
}

#[cfg(windows)]
mod os {
    /// Windows sends no hang-up.
    pub fn ignore_hangup() {}

    /// The command starts in a process group of its own.
    pub fn own_process_group(_command: &mut std::process::Command) {}

    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_VIRTUAL_TERMINAL_INPUT, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
    };

    /// Columns and rows of the console window.
    pub fn size() -> (u16, u16) {
        // SAFETY: a zeroed info struct filled by the call.
        unsafe {
            let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info);
            let w = info.srWindow;
            ((w.Right - w.Left + 1) as u16, (w.Bottom - w.Top + 1) as u16)
        }
    }

    /// Raw input with virtual terminal sequences.
    pub fn raw_input() {
        // SAFETY: the console mode of standard input, read and written back.
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            GetConsoleMode(handle, &mut mode);
            mode &= !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT);
            mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
            SetConsoleMode(handle, mode);
        }
    }
}
