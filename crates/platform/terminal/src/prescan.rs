//! The pre-scan of the PTY output for the sequences the emulator ignores
//! (ADR 0014): OSC 1 (the icon name, a title as the reference's emulator
//! takes it), OSC 7 (working directory), OSC 133 (prompt and command
//! marks, FinalTerm / iTerm2 style) and Pyoxide's own integration events.
//! The bytes still go to the emulator unchanged; the scanner only reports
//! where a recognised sequence ends, so the session can note the cursor
//! position at that point.

use std::path::PathBuf;

/// The OSC number of Pyoxide's own shell integration events:
/// `ESC ] 7701 ; <payload> BEL`. The payload is up to the integration
/// scripts.
pub const OWN_OSC: &str = "7701";

/// The longest OSC payload the scanner keeps; longer ones are skipped.
const MAX_OSC: usize = 4096;

/// A recognised sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sequence {
    /// OSC 1: the icon name, which the reference's emulator takes as the
    /// title like OSC 0 and 2 (JediTerm's `doProcessOsc`); shells set it
    /// to the running command's name after the command line (OSC 2).
    IconName(String),
    /// OSC 7: the shell's working directory.
    WorkingDirectory(PathBuf),
    /// OSC 133.
    Mark(MarkKind),
    /// Pyoxide's own event ([`OWN_OSC`]) with its payload.
    Own(String),
}

/// The OSC 133 marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// `A`: the prompt starts.
    PromptStart,
    /// `B`: the prompt ends, the command line starts.
    CommandStart,
    /// `C`: the command was entered and runs.
    CommandExecuted,
    /// `D[;code]`: the command finished.
    CommandFinished { exit_code: Option<i32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    Osc,
    /// ESC inside an OSC: `\` ends it, anything else aborts it.
    OscEscape,
}

/// A byte-level scanner that keeps its state between chunks.
#[derive(Debug)]
pub struct Scanner {
    state: State,
    osc: Vec<u8>,
    overflow: bool,
    windows: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new(cfg!(windows))
    }
}

impl Scanner {
    /// `windows`: OSC 7 paths are Windows paths (`/C:/x` -> `C:\x`).
    pub fn new(windows: bool) -> Self {
        Self {
            state: State::Ground,
            osc: Vec::new(),
            overflow: false,
            windows,
        }
    }

    /// Scans `bytes` until the end of the next recognised sequence and
    /// returns the number of bytes consumed (up to and including its
    /// terminator) with the sequence, or `None` once all of `bytes` was
    /// consumed without one.
    pub fn scan(&mut self, bytes: &[u8]) -> Option<(usize, Sequence)> {
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            match self.state {
                State::Ground => {
                    // Fast path: skip to the next ESC (none: all consumed).
                    i += bytes[i - 1..].iter().position(|&b| b == 0x1b)?;
                    self.state = State::Escape;
                }
                State::Escape => {
                    self.state = if b == b']' {
                        self.osc.clear();
                        self.overflow = false;
                        State::Osc
                    } else if b == 0x1b {
                        State::Escape
                    } else {
                        State::Ground
                    };
                }
                State::Osc => match b {
                    0x07 => {
                        self.state = State::Ground;
                        if let Some(seq) = self.finish() {
                            return Some((i, seq));
                        }
                    }
                    0x1b => self.state = State::OscEscape,
                    0x18 | 0x1a => self.state = State::Ground,
                    _ => {
                        if self.osc.len() < MAX_OSC {
                            self.osc.push(b);
                        } else {
                            self.overflow = true;
                        }
                    }
                },
                State::OscEscape => {
                    if b == b'\\' {
                        self.state = State::Ground;
                        if let Some(seq) = self.finish() {
                            return Some((i, seq));
                        }
                    } else if b == b']' {
                        // An unterminated OSC followed by a new one.
                        self.osc.clear();
                        self.overflow = false;
                        self.state = State::Osc;
                    } else {
                        self.state = State::Ground;
                    }
                }
            }
        }
        None
    }

    fn finish(&mut self) -> Option<Sequence> {
        if self.overflow {
            return None;
        }
        let text = std::str::from_utf8(&self.osc).ok()?;
        let (number, rest) = text.split_once(';').unwrap_or((text, ""));
        match number {
            "1" => Some(Sequence::IconName(rest.to_owned())),
            "7" => parse_osc7(rest, self.windows).map(Sequence::WorkingDirectory),
            "133" => parse_osc133(rest).map(Sequence::Mark),
            OWN_OSC => Some(Sequence::Own(rest.to_owned())),
            _ => None,
        }
    }
}

/// `file://host/path` with percent-encoding to a path. The host is not
/// checked: the shell runs on this machine.
pub fn parse_osc7(uri: &str, windows: bool) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let decoded = percent_decode(path)?;
    if windows {
        let bytes = decoded.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        let path = if drive { &decoded[1..] } else { &decoded[..] };
        return Some(PathBuf::from(path.replace('/', "\\")));
    }
    Some(PathBuf::from(decoded))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|_| bytes[i] == b'%')
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn parse_osc133(rest: &str) -> Option<MarkKind> {
    let mut parts = rest.split(';');
    Some(match parts.next()? {
        "A" => MarkKind::PromptStart,
        "B" => MarkKind::CommandStart,
        "C" => MarkKind::CommandExecuted,
        "D" => MarkKind::CommandFinished {
            exit_code: parts.next().and_then(|code| code.trim().parse().ok()),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(scanner: &mut Scanner, mut bytes: &[u8]) -> Vec<(usize, Sequence)> {
        let mut out = Vec::new();
        let mut base = 0;
        while let Some((n, seq)) = scanner.scan(bytes) {
            out.push((base + n, seq));
            base += n;
            bytes = &bytes[n..];
        }
        out
    }

    #[test]
    fn finds_osc7_and_133_with_both_terminators() {
        let mut s = Scanner::new(false);
        let parts: [&[u8]; 6] = [
            b"ab\x1b]7;file://host/tmp/a%20b\x07",
            b"cd\x1b]133;D;2\x1b\\",
            b"e\x1b]133;A\x07",
            b"\x1b]133;B\x07",
            b"\x1b]133;D\x07",
            b"tail",
        ];
        let input = parts.concat();
        let ends: Vec<usize> = parts
            .iter()
            .scan(0, |end, p| {
                *end += p.len();
                Some(*end)
            })
            .collect();
        assert_eq!(
            scan_all(&mut s, &input),
            vec![
                (
                    ends[0],
                    Sequence::WorkingDirectory(PathBuf::from("/tmp/a b"))
                ),
                (
                    ends[1],
                    Sequence::Mark(MarkKind::CommandFinished { exit_code: Some(2) })
                ),
                (ends[2], Sequence::Mark(MarkKind::PromptStart)),
                (ends[3], Sequence::Mark(MarkKind::CommandStart)),
                (
                    ends[4],
                    Sequence::Mark(MarkKind::CommandFinished { exit_code: None })
                ),
            ]
        );
    }

    #[test]
    fn keeps_state_across_chunks() {
        let mut s = Scanner::new(false);
        let input = b"x\x1b]133;C\x07y";
        for split in 0..input.len() {
            let mut s2 = Scanner::new(false);
            let (a, b) = input.split_at(split);
            let mut found = scan_all(&mut s2, a);
            found.extend(
                scan_all(&mut s2, b)
                    .into_iter()
                    .map(|(n, q)| (n + split, q)),
            );
            assert_eq!(
                found,
                vec![(9, Sequence::Mark(MarkKind::CommandExecuted))],
                "split at {split}"
            );
        }
        assert!(scan_all(&mut s, b"plain text \x1b[31mred").is_empty());
    }

    #[test]
    fn ignores_other_and_broken_sequences() {
        let mut s = Scanner::new(false);
        assert!(scan_all(&mut s, b"\x1b]0;title\x07\x1b]133;Z\x07\x1b]133;A\x18\x07").is_empty());
        assert_eq!(
            scan_all(&mut s, b"\x1b]2;sleep 100\x07\x1b]1;sleep\x07"),
            [(24, Sequence::IconName("sleep".into()))]
        );
        let long = format!("\x1b]{OWN_OSC};{}\x07", "x".repeat(MAX_OSC + 1));
        assert!(scan_all(&mut s, long.as_bytes()).is_empty());
        let own = format!("\x1b]{OWN_OSC};event=1\x07");
        assert_eq!(
            scan_all(&mut s, own.as_bytes()),
            vec![(own.len(), Sequence::Own("event=1".into()))]
        );
    }

    #[test]
    fn osc7_paths() {
        assert_eq!(
            parse_osc7("file:///Users/me/a%2Fb", false),
            Some(PathBuf::from("/Users/me/a/b"))
        );
        assert_eq!(
            parse_osc7("file://pc/C:/Users/me/%D0%BF", true),
            Some(PathBuf::from("C:\\Users\\me\\п"))
        );
        assert_eq!(parse_osc7("http://x/y", false), None);
        assert_eq!(
            parse_osc7("file://host/bad%2", false),
            Some(PathBuf::from("/bad%2"))
        );
        assert_eq!(
            parse_osc7("file://host/%zz", false),
            Some(PathBuf::from("/%zz"))
        );
    }
}
