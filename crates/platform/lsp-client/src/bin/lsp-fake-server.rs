//! A language server driven by a script, for the tests of `lsp-client`.
//!
//! The script is a JSON array of steps in the environment variable
//! `LSP_FAKE_SCRIPT`, or a file of one JSON step per line given as the
//! first argument. Steps run in order:
//!
//! | step | does |
//! |---|---|
//! | `{"expect": method, "params": p, "within": ms}` | waits (at most `within`, default 30 s) for a message with this method whose params contain `p` (see below); messages that do not match are kept for later steps |
//! | `{"reply": result, "to": method}` | answers the last expected request (or the last one with `method`) |
//! | `{"reply_error": {"code": c, "message": m}, "to": method}` | answers with an error |
//! | `{"notify": method, "params": p}` | sends a notification |
//! | `{"request": method, "params": p, "id": id}` | sends a request to the client |
//! | `{"expect_response": id, "result": r}` or `"error": e` | waits for the client's answer to a request |
//! | `{"forbid": method, "for": ms}` | fails when a message with this method comes within `ms` |
//! | `{"log": text}` | sends `window/logMessage` (type Log) with `text`: a marker tests wait for |
//! | `{"ignore": [methods]}` | drops these methods from now on instead of keeping them |
//! | `{"delay": ms}` | waits |
//! | `{"stderr": text}` | writes to standard error |
//! | `{"raw": [parts], "pause": ms}` | writes the parts (texts, or arrays of bytes) to standard output as they are, pausing between them |
//! | `{"exit": code}` | exits at once |
//! | `{"hang": true}` | stops reading and answering, forever |
//! | `{"mirror": encoding}` | from now on keeps the open documents as a server does (`didOpen`, incremental or full `didChange` in `utf-8`, `utf-16` or `utf-32` positions, `didClose`) and after each change publishes one diagnostic whose message is the whole text; ends like the idle state below |
//!
//! Expected values match when every member they name matches (objects),
//! element by element (arrays) or equal (others); `"$any"` matches
//! anything, `"$absent"` a missing member, `"$last_id"` the id of the last
//! expected request, `"$id:<method>"` the id of the last request with that
//! method.
//!
//! After the script it idles: `shutdown` is answered with `null`, `exit`
//! and the end of input end the process with code 0, other requests get a
//! method-not-found error. A failed step writes the reason to standard
//! error and exits with code 101.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "../framing.rs"]
mod framing;

/// How long a step waits for a message by default.
const WAIT: Duration = Duration::from_secs(30);

fn main() {
    let script = load();
    let mut fake = Fake::new();
    for step in &script {
        fake.step(step);
    }
    fake.idle(None);
}

fn load() -> Vec<Value> {
    if let Ok(text) = std::env::var("LSP_FAKE_SCRIPT") {
        return match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(steps)) => steps,
            _ => fail("LSP_FAKE_SCRIPT is not a JSON array"),
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
    eprintln!("lsp-fake-server: {reason}");
    std::process::exit(101)
}

/// What reading gave.
enum Input {
    Message(Value),
    End,
    Timeout,
}

struct Fake {
    input: mpsc::Receiver<Value>,
    /// Messages read but not matched yet.
    deferred: VecDeque<Value>,
    ignore: HashSet<String>,
    last_id: Option<Value>,
    ids: HashMap<String, Value>,
}

impl Fake {
    fn new() -> Self {
        let (tx, input) = mpsc::channel();
        // Standard input on a thread of its own, so steps can wait with a
        // limit.
        std::thread::spawn(move || {
            let mut decoder = framing::FrameDecoder::new();
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 8192];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => decoder.push(&buf[..n]),
                }
                while let Some(frame) = decoder.next_frame() {
                    match frame {
                        Ok(message) => {
                            if tx.send(message).is_err() {
                                return;
                            }
                        }
                        Err(error) => fail(&format!("malformed input: {error}")),
                    }
                }
            }
        });
        Self {
            input,
            deferred: VecDeque::new(),
            ignore: HashSet::new(),
            last_id: None,
            ids: HashMap::new(),
        }
    }

    fn read(&mut self, deadline: Option<Instant>) -> Input {
        let received = match deadline {
            Some(deadline) => match self
                .input
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(message) => Ok(message),
                Err(mpsc::RecvTimeoutError::Timeout) => return Input::Timeout,
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
            },
            None => self.input.recv().map_err(|_| ()),
        };
        match received {
            Ok(message) => Input::Message(message),
            Err(()) => Input::End,
        }
    }

    fn step(&mut self, step: &Value) {
        let text = |key: &str| step.get(key).and_then(Value::as_str);
        let millis = |key: &str| {
            step.get(key)
                .and_then(Value::as_u64)
                .map(Duration::from_millis)
        };
        if let Some(method) = text("expect") {
            let params = step.get("params");
            let within = millis("within").unwrap_or(WAIT);
            let message = self.expect(method, within, |fake, m| {
                m.get("method").and_then(Value::as_str) == Some(method)
                    && params
                        .is_none_or(|p| fake.matches(p, m.get("params").unwrap_or(&Value::Null)))
            });
            if let Some(id) = message.get("id") {
                self.last_id = Some(id.clone());
                self.ids.insert(method.to_owned(), id.clone());
            }
        } else if let Some(result) = step.get("reply") {
            let id = self.reply_id(text("to"));
            send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        } else if let Some(error) = step.get("reply_error") {
            let id = self.reply_id(text("to"));
            send(&json!({"jsonrpc": "2.0", "id": id, "error": error}));
        } else if let Some(method) = text("notify") {
            let params = step.get("params").cloned().unwrap_or(Value::Null);
            send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
        } else if let Some(method) = text("request") {
            let params = step.get("params").cloned().unwrap_or(Value::Null);
            let id = step.get("id").cloned().unwrap_or(json!(1));
            send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        } else if let Some(id) = step.get("expect_response") {
            let message = self.expect(&format!("an answer to {id}"), WAIT, |_, m| {
                m.get("method").is_none() && m.get("id") == Some(id)
            });
            for key in ["result", "error"] {
                if let Some(expected) = step.get(key) {
                    let actual = message.get(key).unwrap_or(&Value::Null);
                    if !self.matches(expected, actual) {
                        fail(&format!(
                            "answer {id}: expected {key} {expected}, got {message}"
                        ));
                    }
                }
            }
        } else if let Some(method) = text("forbid") {
            let deadline = Instant::now() + millis("for").unwrap_or(Duration::from_millis(500));
            if let Some(found) = self.deferred.iter().find(|m| is(m, method)) {
                fail(&format!("unexpected {found}"));
            }
            loop {
                match self.read(Some(deadline)) {
                    Input::Message(message) if is(&message, method) => {
                        fail(&format!("unexpected {message}"))
                    }
                    Input::Message(message) => self.keep(message),
                    Input::End | Input::Timeout => break,
                }
            }
        } else if let Some(text) = text("log") {
            send(
                &json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"type": 4, "message": text}}),
            );
        } else if let Some(methods) = step.get("ignore").and_then(Value::as_array) {
            self.ignore
                .extend(methods.iter().filter_map(Value::as_str).map(str::to_owned));
        } else if let Some(delay) = millis("delay") {
            std::thread::sleep(delay);
        } else if let Some(text) = text("stderr") {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(text.as_bytes());
            let _ = stderr.flush();
        } else if let Some(parts) = step.get("raw").and_then(Value::as_array) {
            let pause = step.get("pause").and_then(Value::as_u64).unwrap_or(0);
            for part in parts {
                let bytes: Vec<u8> = match part {
                    Value::String(text) => text.as_bytes().to_vec(),
                    Value::Array(bytes) => bytes
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|b| b as u8)
                        .collect(),
                    _ => fail(&format!("raw output {part}")),
                };
                write_raw(&bytes);
                std::thread::sleep(Duration::from_millis(pause));
            }
        } else if let Some(code) = step.get("exit").and_then(Value::as_i64) {
            std::process::exit(code as i32);
        } else if step.get("hang").is_some() {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        } else if let Some(encoding) = text("mirror") {
            let mut mirror = Mirror {
                encoding: encoding.to_owned(),
                docs: HashMap::new(),
            };
            self.idle(Some(&mut mirror));
        } else {
            fail(&format!("unknown step {step}"));
        }
    }

    fn reply_id(&self, to: Option<&str>) -> Value {
        let id = match to {
            Some(method) => self.ids.get(method),
            None => self.last_id.as_ref(),
        };
        id.cloned()
            .unwrap_or_else(|| fail(&format!("no request to answer ({to:?})")))
    }

    fn keep(&mut self, message: Value) {
        let ignored = message
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|m| self.ignore.contains(m));
        if !ignored {
            self.deferred.push_back(message);
        }
    }

    /// The first message, kept or new, that `wanted` accepts.
    fn expect(
        &mut self,
        what: &str,
        within: Duration,
        wanted: impl Fn(&Fake, &Value) -> bool,
    ) -> Value {
        if let Some(i) = self.deferred.iter().position(|m| wanted(self, m)) {
            return self.deferred.remove(i).expect("an index in range");
        }
        let deadline = Instant::now() + within;
        loop {
            let message = match self.read(Some(deadline)) {
                Input::Message(message) => message,
                Input::End | Input::Timeout => {
                    let kept: Vec<String> = self.deferred.iter().map(Value::to_string).collect();
                    fail(&format!(
                        "no {what} came; received meanwhile: {}",
                        kept.join(", ")
                    ));
                }
            };
            if wanted(self, &message) {
                return message;
            }
            self.keep(message);
        }
    }

    fn matches(&self, expected: &Value, actual: &Value) -> bool {
        match expected {
            Value::String(s) if s == "$any" => true,
            Value::String(s) if s == "$last_id" => self.last_id.as_ref() == Some(actual),
            Value::String(s) if s.starts_with("$id:") => self.ids.get(&s[4..]) == Some(actual),
            Value::Object(expected) => actual.as_object().is_some_and(|actual| {
                expected.iter().all(|(key, value)| match actual.get(key) {
                    None => *value == "$absent",
                    Some(found) => *value != "$absent" && self.matches(value, found),
                })
            }),
            Value::Array(expected) => actual.as_array().is_some_and(|actual| {
                actual.len() == expected.len()
                    && expected.iter().zip(actual).all(|(e, a)| self.matches(e, a))
            }),
            other => other == actual,
        }
    }

    /// After the script (or in mirror mode): answer `shutdown`, end on
    /// `exit` or end of input.
    fn idle(&mut self, mut mirror: Option<&mut Mirror>) -> ! {
        loop {
            let message = match self.deferred.pop_front() {
                Some(message) => message,
                None => match self.read(None) {
                    Input::Message(message) => message,
                    Input::End | Input::Timeout => std::process::exit(0),
                },
            };
            let method = message.get("method").and_then(Value::as_str);
            let id = message.get("id").cloned();
            match (method, id) {
                (Some("exit"), _) => std::process::exit(0),
                (Some("shutdown"), Some(id)) => {
                    send(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                }
                (Some(method), Some(id)) => send(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("the script does not handle {method}")},
                })),
                (Some(method), None) => {
                    if let Some(mirror) = mirror.as_deref_mut() {
                        mirror.notification(method, message.get("params").unwrap_or(&Value::Null));
                    }
                }
                _ => {}
            }
        }
    }
}

fn is(message: &Value, method: &str) -> bool {
    message.get("method").and_then(Value::as_str) == Some(method)
}

/// The documents as a server keeps them.
struct Mirror {
    encoding: String,
    docs: HashMap<String, String>,
}

impl Mirror {
    fn notification(&mut self, method: &str, params: &Value) {
        let uri = params
            .get("textDocument")
            .and_then(|d| d.get("uri"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let version = params.get("textDocument").and_then(|d| d.get("version"));
        match method {
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"].as_str().unwrap_or_default();
                self.docs.insert(uri.clone(), text.to_owned());
            }
            "textDocument/didChange" => {
                let Some(text) = self.docs.get_mut(&uri) else {
                    fail(&format!("a change of {uri}, which is not open"));
                };
                for change in params["contentChanges"].as_array().into_iter().flatten() {
                    let new_text = change["text"].as_str().unwrap_or_default();
                    match change.get("range") {
                        Some(range) => {
                            let start = offset(text, &range["start"], &self.encoding);
                            let end = offset(text, &range["end"], &self.encoding);
                            if start > end {
                                fail(&format!("a change from {start} to {end}"));
                            }
                            text.replace_range(start..end, new_text);
                        }
                        None => *text = new_text.to_owned(),
                    }
                }
            }
            "textDocument/didClose" => {
                self.docs.remove(&uri);
                return;
            }
            _ => return,
        }
        let text = &self.docs[&uri];
        send(
            &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                "uri": uri,
                "version": version,
                "diagnostics": [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                    "message": text,
                    "source": "mirror",
                }],
            }}),
        );
    }
}

/// The byte offset of an LSP position; an invalid one fails the server.
fn offset(text: &str, position: &Value, encoding: &str) -> usize {
    let line = position["line"].as_u64().unwrap_or(u64::MAX) as usize;
    let character = position["character"].as_u64().unwrap_or(u64::MAX) as usize;
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => fail(&format!("line {line} is past the end of {text:?}")),
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let mut units = 0;
    for (i, c) in text[start..end].char_indices() {
        if units == character {
            return start + i;
        }
        units += match encoding {
            "utf-8" => c.len_utf8(),
            "utf-32" => 1,
            _ => c.len_utf16(),
        };
        if units > character {
            fail(&format!(
                "column {character} is inside a character of line {line}"
            ));
        }
    }
    if units == character {
        return end;
    }
    fail(&format!(
        "column {character} is past the end of line {line} ({units} units)"
    ))
}

fn send(message: &Value) {
    let body = serde_json::to_vec(message).expect("JSON");
    write_raw(&framing::encode(&body));
}

fn write_raw(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(bytes)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        // The client closed the pipe.
        std::process::exit(0);
    }
}
