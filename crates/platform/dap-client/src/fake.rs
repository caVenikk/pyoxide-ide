//! A debug adapter driven by a script in the test's own process (feature
//! `fake`), for the tests of crates that use the client and cannot start
//! the `dap-fake-adapter` binary. The steps are those of the binary
//! (`src/script.rs`).

use std::sync::mpsc;
use std::thread;

use process::ExitStatus;
use serde_json::Value;

use crate::script::{self, Io, Stop};
use crate::transport::{Incoming, Transport};

/// A transport to an adapter that runs `script` on a thread of its own.
/// Its exit code is the [`Incoming::Closed`] status: 0 at the end, 101
/// when a step failed (the reason goes to the log, which the client's
/// handler gets as standard error).
pub fn adapter(script: Vec<Value>) -> Transport {
    let (transport, peer) = Transport::memory();
    let (to_script, input) = mpsc::channel();
    let written = peer.written;
    let to_client = peer.to_client;
    let log = to_client.clone();
    thread::Builder::new()
        .name("dap fake input".to_owned())
        .spawn(move || {
            script::decode_into(
                || written.recv().ok(),
                &to_script,
                |error| {
                    let _ = log.send(Incoming::Stderr(error.into_bytes()));
                },
            );
        })
        .expect("a thread for the fake adapter");
    thread::Builder::new()
        .name("dap fake adapter".to_owned())
        .spawn(move || {
            let mut io = Channel(to_client.clone());
            let code = match script::run(&script, input, &mut io) {
                Stop::Exit(code) => code,
                Stop::Fail(reason) => {
                    io.log(&format!("dap-fake-adapter: {reason}\n"));
                    101
                }
            };
            let _ = to_client.send(Incoming::Closed(Some(ExitStatus {
                code: Some(code),
                signal: None,
            })));
        })
        .expect("a thread for the fake adapter");
    transport
}

struct Channel(flume::Sender<Incoming>);

impl Io for Channel {
    fn write(&mut self, bytes: &[u8]) -> bool {
        self.0.send(Incoming::Data(bytes.to_vec())).is_ok()
    }

    fn log(&mut self, text: &str) {
        let _ = self.0.send(Incoming::Stderr(text.as_bytes().to_vec()));
    }
}
