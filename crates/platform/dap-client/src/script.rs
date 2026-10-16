//! A debug adapter driven by a script, for tests: the engine of the
//! `dap-fake-adapter` binary (which includes this file as its own module)
//! and of [`crate::fake`] (the same in the test's process).
//!
//! A script is a list of JSON steps run in order:
//!
//! | step | does |
//! |---|---|
//! | `{"expect": command, "arguments": a, "within": ms}` | waits (at most `within`, default 30 s) for a request with this command whose arguments contain `a` (see below); messages that do not match are kept for later steps |
//! | `{"respond": body, "to": command}` | answers the last expected request (or the last one with `command`) with success and `body` (`null`: none) |
//! | `{"fail": message, "to": command}` | answers with an error whose text is `message` (`"cancelled"` is the protocol's cancelled answer) |
//! | `{"event": name, "body": b}` | sends an event |
//! | `{"request": command, "arguments": a}` | sends a request to the client |
//! | `{"expect_response": command, "body": b, "success": s}` | waits for the client's answer to the last request with this command |
//! | `{"forbid": command, "for": ms}` | fails when a request with this command comes within `ms` |
//! | `{"ignore": [commands]}` | drops these requests from now on instead of keeping them |
//! | `{"delay": ms}` | waits |
//! | `{"stderr": text}` | writes to the log (standard error) |
//! | `{"raw": [parts], "pause": ms}` | writes the parts (texts, or arrays of bytes) to the stream as they are |
//! | `{"exit": code}` | ends at once with this exit code |
//! | `{"hang": true}` | stops reading and answering until the client goes away |
//!
//! Expected values match when every member they name matches (objects),
//! element by element (arrays) or equal (others); `"$any"` matches
//! anything, `"$absent"` a missing member and `"$seq:<command>"` the seq of
//! the last request with that command.
//!
//! After the script it idles: `disconnect` is answered and ends it with
//! code 0, `terminate` is answered and followed by a `terminated` event,
//! `cancel` is answered, other requests get an error; the end of input
//! ends it with code 0. A failed step ends it with code 101 and the reason
//! in the log.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long a step waits for a message by default.
const WAIT: Duration = Duration::from_secs(30);

/// Why the script ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Exit(i32),
    Fail(String),
}

/// Where the script writes.
pub trait Io {
    /// Writes bytes to the message stream; `false` when the client is gone.
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// Writes to the log.
    fn log(&mut self, text: &str);
}

/// Runs `script` over decoded messages from `input` and returns why it
/// ended.
pub fn run(script: &[Value], input: mpsc::Receiver<Value>, io: &mut dyn Io) -> Stop {
    let mut fake = Fake {
        input,
        io,
        deferred: VecDeque::new(),
        ignore: HashSet::new(),
        last: None,
        by_command: HashMap::new(),
        reverse: HashMap::new(),
        seq: 0,
    };
    for step in script {
        if let Err(stop) = fake.step(step) {
            return stop;
        }
    }
    match fake.idle() {
        Ok(()) => Stop::Exit(0),
        Err(stop) => stop,
    }
}

enum Input {
    Message(Value),
    End,
    Timeout,
}

struct Fake<'a> {
    input: mpsc::Receiver<Value>,
    io: &'a mut dyn Io,
    deferred: VecDeque<Value>,
    ignore: HashSet<String>,
    /// The last expected request.
    last: Option<Value>,
    /// The last request with each command.
    by_command: HashMap<String, Value>,
    /// The seq of the last request sent to the client, by command.
    reverse: HashMap<String, i64>,
    seq: i64,
}

fn fail<T>(reason: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Fail(reason.into()))
}

fn command(message: &Value) -> Option<&str> {
    if message.get("type").and_then(Value::as_str) != Some("request") {
        return None;
    }
    message.get("command").and_then(Value::as_str)
}

impl Fake<'_> {
    fn read(&mut self, deadline: Option<Instant>) -> Input {
        let received = match deadline {
            Some(deadline) => match self
                .input
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => return Input::Timeout,
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            },
            None => self.input.recv().ok(),
        };
        match received {
            Some(message) => Input::Message(message),
            None => Input::End,
        }
    }

    fn send(&mut self, mut message: Value) -> Result<(), Stop> {
        self.seq += 1;
        message["seq"] = self.seq.into();
        let body = serde_json::to_vec(&message).expect("JSON");
        if self.io.write(&stream_framing::encode(&body)) {
            Ok(())
        } else {
            Err(Stop::Exit(0))
        }
    }

    fn respond(&mut self, request: &Value, result: Result<Value, String>) -> Result<(), Stop> {
        let mut message = json!({
            "type": "response",
            "request_seq": request["seq"],
            "command": request["command"],
        });
        match result {
            Ok(body) => {
                message["success"] = true.into();
                if !body.is_null() {
                    message["body"] = body;
                }
            }
            Err(text) => {
                message["success"] = false.into();
                message["message"] = text.clone().into();
                if text != "cancelled" {
                    message["body"] = json!({"error": {"id": 1, "format": text}});
                }
            }
        }
        self.send(message)
    }

    fn step(&mut self, step: &Value) -> Result<(), Stop> {
        let text = |key: &str| step.get(key).and_then(Value::as_str);
        let millis = |key: &str| {
            step.get(key)
                .and_then(Value::as_u64)
                .map(Duration::from_millis)
        };
        if let Some(wanted) = text("expect") {
            let arguments = step.get("arguments");
            let within = millis("within").unwrap_or(WAIT);
            let message = self.expect(wanted, within, |fake, m| {
                command(m) == Some(wanted)
                    && arguments
                        .is_none_or(|a| fake.matches(a, m.get("arguments").unwrap_or(&Value::Null)))
            })?;
            self.by_command.insert(wanted.to_owned(), message.clone());
            self.last = Some(message);
        } else if let Some(body) = step.get("respond") {
            let request = self.request_to(text("to"))?;
            self.respond(&request, Ok(body.clone()))?;
        } else if let Some(message) = text("fail") {
            let request = self.request_to(text("to"))?;
            self.respond(&request, Err(message.to_owned()))?;
        } else if let Some(name) = text("event") {
            let mut event = json!({"type": "event", "event": name});
            if let Some(body) = step.get("body") {
                event["body"] = body.clone();
            }
            self.send(event)?;
        } else if let Some(name) = text("request") {
            let arguments = step.get("arguments").cloned().unwrap_or(json!({}));
            self.reverse.insert(name.to_owned(), self.seq + 1);
            self.send(json!({"type": "request", "command": name, "arguments": arguments}))?;
        } else if let Some(name) = text("expect_response") {
            let Some(&seq) = self.reverse.get(name) else {
                return fail(format!("no {name} request was sent"));
            };
            let message = self.expect(&format!("an answer to {name}"), WAIT, |_, m| {
                m.get("type").and_then(Value::as_str) == Some("response")
                    && m.get("request_seq").and_then(Value::as_i64) == Some(seq)
            })?;
            for key in ["body", "success"] {
                if let Some(expected) = step.get(key) {
                    let actual = message.get(key).unwrap_or(&Value::Null);
                    if !self.matches(expected, actual) {
                        return fail(format!(
                            "answer to {name}: expected {key} {expected}, got {message}"
                        ));
                    }
                }
            }
        } else if let Some(name) = text("forbid") {
            let deadline = Instant::now() + millis("for").unwrap_or(Duration::from_millis(500));
            if let Some(found) = self.deferred.iter().find(|m| command(m) == Some(name)) {
                return fail(format!("unexpected {found}"));
            }
            loop {
                match self.read(Some(deadline)) {
                    Input::Message(message) if command(&message) == Some(name) => {
                        return fail(format!("unexpected {message}"));
                    }
                    Input::Message(message) => self.keep(message),
                    Input::End | Input::Timeout => break,
                }
            }
        } else if let Some(names) = step.get("ignore").and_then(Value::as_array) {
            self.ignore
                .extend(names.iter().filter_map(Value::as_str).map(str::to_owned));
        } else if let Some(delay) = millis("delay") {
            std::thread::sleep(delay);
        } else if let Some(text) = text("stderr") {
            self.io.log(text);
        } else if let Some(parts) = step.get("raw").and_then(Value::as_array) {
            let pause = millis("pause").unwrap_or_default();
            for part in parts {
                let bytes: Vec<u8> = match part {
                    Value::String(text) => text.as_bytes().to_vec(),
                    Value::Array(bytes) => bytes
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|b| b as u8)
                        .collect(),
                    _ => return fail(format!("raw output {part}")),
                };
                if !self.io.write(&bytes) {
                    return Err(Stop::Exit(0));
                }
                std::thread::sleep(pause);
            }
        } else if let Some(code) = step.get("exit").and_then(Value::as_i64) {
            return Err(Stop::Exit(code as i32));
        } else if step.get("hang").is_some() {
            while self.input.recv().is_ok() {}
            return Err(Stop::Exit(0));
        } else {
            return fail(format!("unknown step {step}"));
        }
        Ok(())
    }

    fn request_to(&self, to: Option<&str>) -> Result<Value, Stop> {
        let request = match to {
            Some(name) => self.by_command.get(name),
            None => self.last.as_ref(),
        };
        match request {
            Some(request) => Ok(request.clone()),
            None => fail(format!("no request to answer ({to:?})")),
        }
    }

    fn keep(&mut self, message: Value) {
        let ignored = command(&message).is_some_and(|c| self.ignore.contains(c));
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
    ) -> Result<Value, Stop> {
        if let Some(i) = self.deferred.iter().position(|m| wanted(self, m)) {
            return Ok(self.deferred.remove(i).expect("an index in range"));
        }
        let deadline = Instant::now() + within;
        loop {
            let message = match self.read(Some(deadline)) {
                Input::Message(message) => message,
                Input::End | Input::Timeout => {
                    let kept: Vec<String> = self.deferred.iter().map(Value::to_string).collect();
                    return fail(format!(
                        "no {what} came; received meanwhile: {}",
                        kept.join(", ")
                    ));
                }
            };
            if wanted(self, &message) {
                return Ok(message);
            }
            self.keep(message);
        }
    }

    fn matches(&self, expected: &Value, actual: &Value) -> bool {
        match expected {
            Value::String(s) if s == "$any" => true,
            Value::String(s) if s.starts_with("$seq:") => self
                .by_command
                .get(&s[5..])
                .is_some_and(|request| request.get("seq") == Some(actual)),
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

    /// Answers what a quiet adapter answers until `disconnect` or the end
    /// of input.
    fn idle(&mut self) -> Result<(), Stop> {
        loop {
            let message = match self.deferred.pop_front() {
                Some(message) => message,
                None => match self.read(None) {
                    Input::Message(message) => message,
                    Input::End | Input::Timeout => return Ok(()),
                },
            };
            match command(&message) {
                Some("disconnect") => {
                    self.respond(&message, Ok(Value::Null))?;
                    return Ok(());
                }
                Some("terminate") => {
                    self.respond(&message, Ok(Value::Null))?;
                    self.send(json!({"type": "event", "event": "terminated"}))?;
                }
                Some("cancel") => self.respond(&message, Ok(Value::Null))?,
                Some(name) => {
                    let text = format!("the script does not handle {name}");
                    self.respond(&message, Err(text))?;
                }
                None => {}
            }
        }
    }
}

/// Decodes frames from chunks read by `read` (until it returns `None`) and
/// passes the messages on; a malformed frame is reported to `log`.
pub fn decode_into(
    mut read: impl FnMut() -> Option<Vec<u8>>,
    to: &mpsc::Sender<Value>,
    mut log: impl FnMut(String),
) {
    let mut decoder = stream_framing::FrameDecoder::new();
    while let Some(chunk) = read() {
        decoder.push(&chunk);
        while let Some(frame) = decoder.next_frame() {
            match frame {
                Ok(message) => {
                    if to.send(message).is_err() {
                        return;
                    }
                }
                Err(error) => log(format!("malformed input: {error}")),
            }
        }
    }
}
