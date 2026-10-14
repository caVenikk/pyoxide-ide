//! What the tests against the scripted fake server share.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use code_intel::{
    CodeIntel, Diagnostic, DocumentId, Features, IntelEvent, MessageLevel, Point, ServerState,
    ServerStatus,
};
use lsp_client::{ServerSpec, Timeouts};
use process::CommandLine;
use serde_json::{Value, json};
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, SelectionSet};

pub const FAKE: &str = env!("CARGO_BIN_EXE_lsp-fake-server");
pub const WAIT: Duration = Duration::from_secs(20);

/// Short waits, so stopping a server that does not cooperate is quick.
pub const QUICK: Timeouts = Timeouts {
    initialize: Duration::from_millis(1500),
    shutdown: Duration::from_millis(500),
    exit: Duration::from_millis(300),
};

/// A folder of its own for each test, removed at its end.
pub struct TestDir(pub PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn root(test: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("lsp-client-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// The URI of a path without characters that need escaping.
pub fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    match text.as_bytes() {
        [drive, b':', ..] if cfg!(windows) => format!(
            "file:///{}%3A{}",
            (*drive as char).to_ascii_lowercase(),
            &text[2..]
        ),
        _ => format!("file://{text}"),
    }
}

pub fn spec(id: &str, root: &Path, steps: Vec<Value>) -> ServerSpec {
    let command =
        CommandLine::new(FAKE).with_env("LSP_FAKE_SCRIPT", Value::Array(steps).to_string());
    ServerSpec::new(id, command, root)
        .with_languages(["python"])
        .with_features(Features::all())
}

/// `initialize` answered with `result`, then `initialized`.
pub fn handshake(result: Value) -> Vec<Value> {
    vec![
        json!({"expect": "initialize"}),
        json!({"reply": result}),
        json!({"expect": "initialized", "params": {}}),
    ]
}

pub fn steps(parts: impl IntoIterator<Item = Vec<Value>>) -> Vec<Value> {
    parts.into_iter().flatten().collect()
}

pub fn r(l1: u32, c1: u32, l2: u32, c2: u32) -> Value {
    json!({"start": {"line": l1, "character": c1}, "end": {"line": l2, "character": c2}})
}

/// The events of the services, waited for one by one. Servers run on
/// threads of their own: events of different servers come in any order,
/// so waits for several servers take them in any order
/// ([`Events::running_all`], [`Events::terminated_all`]).
pub struct Events(pub flume::Receiver<IntelEvent>);

impl Events {
    pub fn of(intel: &dyn CodeIntel) -> Self {
        Self(intel.events())
    }

    /// The first event `wanted` accepts, skipping others; a termination
    /// it does not accept fails the test.
    pub fn next<T>(&self, what: &str, mut wanted: impl FnMut(&IntelEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        let mut skipped = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(event) = self.0.recv_timeout(left) else {
                panic!("no {what} within {WAIT:?}; skipped: {skipped:#?}");
            };
            if let Some(found) = wanted(&event) {
                return found;
            }
            if let IntelEvent::Status(ServerStatus {
                id,
                state: ServerState::Terminated { error_output },
                ..
            }) = &event
            {
                panic!("{id} terminated while waiting for {what}: {error_output}");
            }
            skipped.push(event);
        }
    }

    /// The next status of server `id` that `accept` takes.
    pub fn status_where(&self, id: &str, accept: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
        self.next(&format!("status of {id}"), |event| match event {
            IntelEvent::Status(status) if status.id.as_str() == id && accept(status) => {
                Some(status.clone())
            }
            _ => None,
        })
    }

    /// The next status of server `id` in a state `state` takes.
    pub fn status(&self, id: &str, state: impl Fn(&ServerState) -> bool) -> ServerStatus {
        self.status_where(id, |status| state(&status.state))
    }

    pub fn running(&self, id: &str) -> ServerStatus {
        self.status(id, |s| *s == ServerState::Running)
    }

    /// Waits until every one of the servers is Running, in any order.
    pub fn running_all(&self, ids: &[&str]) -> BTreeMap<String, ServerStatus> {
        self.all(ids, "running", |status| {
            status.state == ServerState::Running
        })
    }

    /// Waits until every one of the servers has a document open, in any
    /// order.
    pub fn opened_all(&self, ids: &[&str]) -> BTreeMap<String, ServerStatus> {
        self.all(ids, "with a document", |status| {
            !status.documents.is_empty()
        })
    }

    /// Waits until a status of each server that `accept` takes came, in any
    /// order.
    pub fn all(
        &self,
        ids: &[&str],
        what: &str,
        accept: impl Fn(&ServerStatus) -> bool,
    ) -> BTreeMap<String, ServerStatus> {
        let mut waiting: BTreeSet<&str> = ids.iter().copied().collect();
        let mut found = BTreeMap::new();
        while !waiting.is_empty() {
            let status = self.next(&format!("{waiting:?} {what}"), |event| match event {
                IntelEvent::Status(status)
                    if waiting.contains(status.id.as_str()) && accept(status) =>
                {
                    Some(status.clone())
                }
                _ => None,
            });
            waiting.remove(status.id.as_str());
            found.insert(status.id.to_string(), status);
        }
        found
    }

    pub fn terminated(&self, id: &str) -> String {
        self.terminated_all(&[id])
            .remove(id)
            .expect("an error output")
    }

    /// Waits until every one of the servers is Terminated, in any order:
    /// their error output.
    pub fn terminated_all(&self, ids: &[&str]) -> BTreeMap<String, String> {
        let mut waiting: BTreeSet<&str> = ids.iter().copied().collect();
        let mut found = BTreeMap::new();
        while !waiting.is_empty() {
            let (id, error) = self.next(&format!("{waiting:?} terminated"), |event| match event {
                IntelEvent::Status(ServerStatus {
                    id,
                    state: ServerState::Terminated { error_output },
                    ..
                }) if waiting.contains(id.as_str()) => Some((id.to_string(), error_output.clone())),
                _ => None,
            });
            waiting.remove(id.as_str());
            found.insert(id, error);
        }
        found
    }

    /// Waits for a `window/logMessage` marker of a script.
    pub fn log(&self, text: &str) {
        self.next(&format!("log {text:?}"), |event| match event {
            IntelEvent::Message {
                level: MessageLevel::Log,
                text: t,
                ..
            } if t == text => Some(()),
            _ => None,
        })
    }

    /// The diagnostics of `doc` in `version` that `accept` takes.
    pub fn diagnostics(
        &self,
        doc: &DocumentId,
        version: u64,
        accept: impl Fn(&[Diagnostic]) -> bool,
    ) -> Vec<Diagnostic> {
        self.next(
            &format!("diagnostics of version {version}"),
            |event| match event {
                IntelEvent::Diagnostics {
                    doc: d,
                    version: v,
                    items,
                } if d == doc && *v == version && accept(items) => Some(items.clone()),
                _ => None,
            },
        )
    }
}

/// A document in an editor: a buffer and the version reported.
pub struct Editor {
    pub doc: DocumentId,
    pub buffer: Buffer,
    pub version: u64,
}

impl Editor {
    pub fn open(intel: &dyn CodeIntel, path: PathBuf, text: &str) -> Self {
        let editor = Self {
            doc: DocumentId::new(path),
            buffer: Buffer::new(text),
            version: 1,
        };
        intel.did_open(&editor.doc, "python", 1, editor.buffer.rope());
        editor
    }

    pub fn edit(&mut self, intel: &dyn CodeIntel, edits: Vec<Edit>) -> Vec<AppliedEdit> {
        self.edit_as(intel, edits, EditKind::Other)
    }

    pub fn edit_as(
        &mut self,
        intel: &dyn CodeIntel,
        edits: Vec<Edit>,
        kind: EditKind,
    ) -> Vec<AppliedEdit> {
        let applied = self.buffer.transact(
            &mut SelectionSet::caret(0),
            edits,
            kind,
            CaretPlacement::AfterEachEdit,
        );
        self.report(intel, &applied);
        applied
    }

    /// Reports applied edits (of an edit, undo or redo).
    pub fn report(&mut self, intel: &dyn CodeIntel, applied: &[AppliedEdit]) {
        if applied.is_empty() {
            return;
        }
        self.version += 1;
        intel.did_change(&self.doc, self.version, applied, self.buffer.rope());
    }

    pub fn offset(&self, needle: &str) -> usize {
        let byte = self.buffer.text().find(needle).expect("in the text");
        self.buffer.rope().byte_to_char(byte)
    }

    pub fn point(&self, needle: &str) -> Point {
        self.buffer.offset_to_point(self.offset(needle))
    }
}

/// A small deterministic generator (xorshift64*); no extra dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}
