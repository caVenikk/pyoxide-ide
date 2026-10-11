//! Prompters for command-line front-ends.

use std::io::{BufRead, IsTerminal};

use crate::{Prompter, Secret};

/// Asks on the terminal without echo; `None` when stdin is not a terminal.
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn secret(&self, prompt: &str) -> Option<Secret> {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        rpassword::prompt_password(prompt).ok().map(Secret::new)
    }
}

/// Reads the secret from one line of stdin (for piping it in).
pub struct StdinPrompter;

impl Prompter for StdinPrompter {
    fn secret(&self, _: &str) -> Option<Secret> {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok()?;
        Some(Secret::new(line.trim_end_matches(['\r', '\n'])))
    }
}
