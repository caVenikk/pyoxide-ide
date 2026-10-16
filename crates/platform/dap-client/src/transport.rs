//! How the client reaches an adapter: an [`Outlet`] for the frames it
//! writes and an [`Inlet`] for what comes back. Standard streams of an
//! adapter process now ([`Transport::stdio`]); a socket later (attach,
//! subprocesses) is another pair of implementations.

use std::io;

use process::{CommandLine, ExitStatus, ProcessEvent, ProcessHandle};

/// What the adapter's side produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// Bytes of the message stream, in any chunks.
    Data(Vec<u8>),
    /// A chunk of the adapter's standard error (its log).
    Stderr(Vec<u8>),
    /// The stream ended; the exit status when it was a process. The last
    /// item.
    Closed(Option<ExitStatus>),
}

/// Where the client's frames go. Writes queue and return at once.
pub trait Outlet: Send + 'static {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// No more input: the adapter reads end of file.
    fn close(&mut self);
}

/// What the adapter sends, read on the client's reader thread.
pub trait Inlet: Send + 'static {
    /// Blocks until something comes; after [`Incoming::Closed`] it is not
    /// called again.
    fn recv(&mut self) -> Incoming;
}

/// A connection to one adapter.
pub struct Transport {
    pub(crate) outlet: Box<dyn Outlet>,
    pub(crate) inlet: Box<dyn Inlet>,
    pub(crate) process: Option<ProcessHandle>,
}

impl Transport {
    pub fn new(outlet: impl Outlet, inlet: impl Inlet) -> Self {
        Self {
            outlet: Box::new(outlet),
            inlet: Box::new(inlet),
            process: None,
        }
    }

    /// Starts an adapter process and talks to it over its standard input
    /// and output. Its standard error is its log.
    pub fn stdio(command: &CommandLine) -> io::Result<Self> {
        let (handle, events) = process::spawn(command)?;
        Ok(Self {
            outlet: Box::new(ProcessOutlet(handle.clone())),
            inlet: Box::new(ProcessInlet(events)),
            process: Some(handle),
        })
    }

    /// The adapter's process, when it is one.
    pub fn process(&self) -> Option<&ProcessHandle> {
        self.process.as_ref()
    }

    /// A transport in memory and its other end, for tests and adapters in
    /// the same process.
    pub fn memory() -> (Transport, MemoryPeer) {
        let (written_tx, written) = flume::unbounded();
        let (to_client, from_peer) = flume::unbounded();
        let transport = Transport::new(
            MemoryOutlet {
                written: Some(written_tx),
            },
            MemoryInlet(from_peer),
        );
        (transport, MemoryPeer { written, to_client })
    }
}

/// The adapter's end of [`Transport::memory`].
pub struct MemoryPeer {
    /// What the client wrote; disconnected when it closed its output.
    pub written: flume::Receiver<Vec<u8>>,
    /// What the client reads.
    pub to_client: flume::Sender<Incoming>,
}

struct ProcessOutlet(ProcessHandle);

impl Outlet for ProcessOutlet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_stdin(bytes)
    }

    fn close(&mut self) {
        self.0.close_stdin();
    }
}

struct ProcessInlet(flume::Receiver<ProcessEvent>);

impl Inlet for ProcessInlet {
    fn recv(&mut self) -> Incoming {
        match self.0.recv() {
            Ok(ProcessEvent::Stdout(chunk)) => Incoming::Data(chunk),
            Ok(ProcessEvent::Stderr(chunk)) => Incoming::Stderr(chunk),
            Ok(ProcessEvent::Exited(status)) => Incoming::Closed(Some(status)),
            Err(_) => Incoming::Closed(None),
        }
    }
}

struct MemoryOutlet {
    written: Option<flume::Sender<Vec<u8>>>,
}

impl Outlet for MemoryOutlet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &self.written {
            Some(tx) => tx
                .send(bytes.to_vec())
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe)),
            None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    fn close(&mut self) {
        self.written = None;
    }
}

struct MemoryInlet(flume::Receiver<Incoming>);

impl Inlet for MemoryInlet {
    fn recv(&mut self) -> Incoming {
        self.0.recv().unwrap_or(Incoming::Closed(None))
    }
}
