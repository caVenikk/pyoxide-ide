//! A debug adapter driven by a script, for the tests of `dap-client`.
//!
//! The script is a JSON array of steps in the environment variable
//! `DAP_FAKE_SCRIPT`, or a file of one JSON step per line given as the
//! first argument; the steps are described in `src/script.rs`. Messages
//! go over standard input and output, the log to standard error.

use std::io::{Read, Write};
use std::sync::mpsc;

use serde_json::Value;

#[path = "../script.rs"]
mod script;

fn main() {
    let steps = load();
    let (tx, input) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 8192];
        script::decode_into(
            || match stdin.read(&mut buf) {
                Ok(0) | Err(_) => None,
                Ok(n) => Some(buf[..n].to_vec()),
            },
            &tx,
            |error| eprintln!("dap-fake-adapter: {error}"),
        );
    });
    match script::run(&steps, input, &mut Stdio) {
        script::Stop::Exit(code) => std::process::exit(code),
        script::Stop::Fail(reason) => fail(&reason),
    }
}

fn load() -> Vec<Value> {
    if let Ok(text) = std::env::var("DAP_FAKE_SCRIPT") {
        return match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(steps)) => steps,
            _ => fail("DAP_FAKE_SCRIPT is not a JSON array"),
        };
    }
    let Some(path) = std::env::args().nth(1) else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| fail(&format!("{line}: {e}"))))
        .collect()
}

fn fail(reason: &str) -> ! {
    eprintln!("dap-fake-adapter: {reason}");
    std::process::exit(101)
}

struct Stdio;

impl script::Io for Stdio {
    fn write(&mut self, bytes: &[u8]) -> bool {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(bytes)
            .and_then(|()| stdout.flush())
            .is_ok()
    }

    fn log(&mut self, text: &str) {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.flush();
    }
}
